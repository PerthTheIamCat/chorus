use chorus_daemon::config::DaemonConfig;
use chorus_daemon::interaction::{
    CreateInteractionRequest, InteractionDecision, InteractionStatus, ResolveInteractionRequest,
    RiskLevel,
};
use chorus_daemon::{DaemonClient, DaemonServer};
use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::{AtomicUsize, Ordering};
use tempfile::tempdir;

static TEST_COUNTER: AtomicUsize = AtomicUsize::new(1);

fn unique_db_path(dir: &std::path::Path) -> std::path::PathBuf {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    dir.join(format!("test_ledger_{id}.db"))
}

#[test]
fn test_socket_creation_and_file_mode_0600() {
    let tmp = tempdir().unwrap();
    let socket_path = tmp.path().join("engine.sock");
    let db_path = unique_db_path(tmp.path());

    let config = DaemonConfig::new()
        .with_socket_path(&socket_path)
        .with_http_addr(None)
        .with_db_path(&db_path);

    let server = DaemonServer::start(config).expect("daemon must start");

    // Check socket file exists
    assert!(socket_path.exists(), "socket file must exist");

    // Check permissions are 0600 (owner only read/write)
    let meta = std::fs::metadata(&socket_path).unwrap();
    let mode = meta.permissions().mode() & 0o777;
    assert_eq!(
        mode, 0o600,
        "socket permissions must be 0600 (owner-only), got {:o}",
        mode
    );

    // Connect using client over Unix domain socket
    let client = DaemonClient::from_uds(&socket_path);
    assert!(client.ping().unwrap(), "ping over Unix socket must succeed");

    let status = client.status().unwrap();
    assert_eq!(status["status"], "running");

    // Stop server and verify socket file is unlinked
    server.stop();
    assert!(!socket_path.exists(), "socket file must be removed on stop");
}

#[test]
fn test_http_fallback_server() {
    let tmp = tempdir().unwrap();
    let socket_path = tmp.path().join("engine.sock");
    let db_path = unique_db_path(tmp.path());

    let config = DaemonConfig::new()
        .with_socket_path(&socket_path)
        .with_http_addr(Some("127.0.0.1:0".to_string()))
        .with_db_path(&db_path);

    let server = DaemonServer::start(config).expect("daemon must start");
    let http_addr = server.http_addr().expect("http addr should be bound");

    // Health is public; host management stays on the owner-only socket.
    let client = DaemonClient::from_http(http_addr);
    assert!(client.ping().unwrap(), "ping over HTTP must succeed");

    assert!(
        client.status().is_err(),
        "TCP status requires authentication"
    );
    let host = DaemonClient::from_uds(&socket_path);
    let session = server.pairing().initiate_session(http_addr).unwrap();
    let device = server
        .pairing()
        .complete_pairing(&session.pairing_token, "test client")
        .unwrap();
    let authorization = format!("Bearer {}", device.auth_token);
    let (code, body) = client
        .request(
            "GET",
            "/lifecycle/status",
            &[("Authorization", &authorization)],
            None,
        )
        .unwrap();
    assert_eq!(code, 200);
    let status: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(status["status"], "running");

    // Test mode switching via HTTP
    assert!(client.set_lifecycle_mode("background").is_err());
    let mode_res = host.set_lifecycle_mode("background").unwrap();
    assert_eq!(mode_res["mode"], "background");

    let status_after = host.status().unwrap();
    assert_eq!(status_after["mode"], "background");

    server.stop();
}

#[test]
fn test_interaction_api_and_resolution() {
    let tmp = tempdir().unwrap();
    let socket_path = tmp.path().join("engine.sock");
    let db_path = unique_db_path(tmp.path());

    let config = DaemonConfig::new()
        .with_socket_path(&socket_path)
        .with_http_addr(None)
        .with_db_path(&db_path);

    let server = DaemonServer::start(config).expect("daemon must start");
    let client = DaemonClient::from_uds(&socket_path);

    // 1. Create interaction
    let create_req = CreateInteractionRequest {
        session_id: Some(1),
        tool: "bash".to_string(),
        kind: "execute".to_string(),
        detail: "cargo test --all".to_string(),
        risk_level: Some(RiskLevel::Medium),
    };
    let created = client.create_interaction(&create_req).unwrap();
    assert!(created.id.starts_with("act_"));
    assert_eq!(created.status, InteractionStatus::Pending);
    assert_eq!(created.tool, "bash");

    // 2. List pending interactions
    let pending_list = client.list_interactions(Some("pending"), None).unwrap();
    assert!(pending_list.iter().any(|i| i.id == created.id));

    // 3. Get single interaction
    let fetched = client.get_interaction(&created.id).unwrap().unwrap();
    assert_eq!(fetched.id, created.id);

    // 4. Resolve interaction
    let resolve_req = ResolveInteractionRequest {
        decision: InteractionDecision::Allow,
        feedback: Some("Approved for CI run".to_string()),
    };
    let resolved = client
        .resolve_interaction(&created.id, &resolve_req)
        .unwrap();
    assert_eq!(resolved.status, InteractionStatus::Allowed);
    assert_eq!(resolved.decision, Some(InteractionDecision::Allow));
    assert_eq!(resolved.feedback.as_deref(), Some("Approved for CI run"));

    // 5. Resolving again must fail with conflict
    let err = client.resolve_interaction(&created.id, &resolve_req);
    assert!(
        err.is_err(),
        "resolving already-resolved interaction must fail"
    );

    server.stop();
}

#[test]
fn test_sse_catchup_replay_and_live_stream() {
    let tmp = tempdir().unwrap();
    let socket_path = tmp.path().join("engine.sock");
    let db_path = unique_db_path(tmp.path());

    // 1. Pre-populate SQLite with events in ledger before daemon starts
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        chorus_core::schema::migrate(&conn).unwrap();
        let sess = chorus_core::ledger::new_session(&conn, "Test Sess", "/tmp").unwrap();

        // Register actor
        conn.execute(
            "INSERT INTO actor (id, name, provider) VALUES (1, 'user', 'local')",
            [],
        )
        .unwrap();

        // Append 3 ledger events
        chorus_core::ledger::append(
            &conn,
            &chorus_core::NewEvent::new(sess, chorus_core::types::EventKind::Msg, 1, "msg 1"),
        )
        .unwrap();
        chorus_core::ledger::append(
            &conn,
            &chorus_core::NewEvent::new(sess, chorus_core::types::EventKind::Msg, 1, "msg 2"),
        )
        .unwrap();
        chorus_core::ledger::append(
            &conn,
            &chorus_core::NewEvent::new(sess, chorus_core::types::EventKind::Msg, 1, "msg 3"),
        )
        .unwrap();
    }

    let config = DaemonConfig::new()
        .with_socket_path(&socket_path)
        .with_http_addr(None)
        .with_db_path(&db_path);

    let server = DaemonServer::start(config).expect("daemon must start");
    let client = DaemonClient::from_uds(&socket_path);

    // 2. Connect with Last-Event-ID: 1 (catch-up replay should yield events 2 and 3)
    let mut stream = client
        .open_events_stream(Some(1), None)
        .expect("must open SSE stream");

    // Event 2 (replayed)
    let ev2 = stream.next_event().unwrap().expect("should receive ev2");
    assert_eq!(ev2.seq, 2);
    assert_eq!(ev2.body, "msg 2");

    // Event 3 (replayed)
    let ev3 = stream.next_event().unwrap().expect("should receive ev3");
    assert_eq!(ev3.seq, 3);
    assert_eq!(ev3.body, "msg 3");

    // 3. Publish live event 4
    let live_ev = client
        .publish_event("Msg", "live event 4", Some(1), Some("user"))
        .unwrap();

    // Stream should receive live event 4 seamlessly
    let ev4 = stream.next_event().unwrap().expect("should receive ev4");
    assert_eq!(ev4.seq, live_ev.seq);
    assert_eq!(ev4.body, "live event 4");

    server.stop();
}

#[test]
fn test_stale_socket_cleanup_and_collision_refusal() {
    let tmp = tempdir().unwrap();
    let socket_path = tmp.path().join("engine.sock");
    let db_path = unique_db_path(tmp.path());

    // Leave a real stale socket, rather than a regular file that must be preserved.
    drop(std::os::unix::net::UnixListener::bind(&socket_path).unwrap());
    assert!(socket_path.exists());

    let config = DaemonConfig::new()
        .with_socket_path(&socket_path)
        .with_http_addr(None)
        .with_db_path(&db_path);

    // Daemon start should detect stale socket, clean it up, and succeed
    let server1 = DaemonServer::start(config.clone()).expect("stale socket must be cleaned");
    assert!(socket_path.exists());

    // Trying to start a second daemon on the same active socket must fail
    let db_path2 = unique_db_path(tmp.path());
    let config2 = DaemonConfig::new()
        .with_socket_path(&socket_path)
        .with_http_addr(None)
        .with_db_path(&db_path2);

    let err = DaemonServer::start(config2);
    assert!(
        matches!(err, Err(chorus_daemon::DaemonError::SocketAlreadyInUse(_))),
        "second daemon on active socket must be refused"
    );

    server1.stop();
}
