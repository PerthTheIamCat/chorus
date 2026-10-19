use chorus_daemon::{config::DaemonConfig, DaemonClient, DaemonServer};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::time::Duration;
use tempfile::{tempdir, TempDir};

fn server() -> (TempDir, DaemonServer) {
    let tmp = tempdir().unwrap();
    let config = DaemonConfig::new()
        .with_socket_path(tmp.path().join("engine.sock"))
        .with_db_path(tmp.path().join("ledger.db"))
        .with_http_addr(Some("127.0.0.1:0".into()));
    let server = DaemonServer::start(config).unwrap();
    (tmp, server)
}

fn raw_request(server: &DaemonServer, request: &[u8]) -> String {
    let mut stream = TcpStream::connect(server.http_addr().unwrap()).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    stream.write_all(request).unwrap();
    let mut response = String::new();
    // Rejected headers may leave unread request bytes and reset the connection.
    match stream.read_to_string(&mut response) {
        Ok(_) => (),
        Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => (),
        Err(e) => panic!("response read failed: {e}"),
    }
    response
}

#[test]
fn unauthenticated_tcp_cannot_read_or_mutate_control_routes() {
    let (_tmp, server) = server();
    let client = DaemonClient::from_http(server.http_addr().unwrap());
    for (method, path) in [
        ("GET", "/events"),
        ("GET", "/v1/events"),
        ("POST", "/events"),
        ("GET", "/interactions"),
        ("POST", "/interactions"),
        ("POST", "/interactions/fake/resolve"),
        ("POST", "/shutdown"),
        ("GET", "/lifecycle/status"),
        ("POST", "/lifecycle/mode"),
        ("GET", "/api/pairing/initiate"),
        ("GET", "/api/pairing/devices"),
        ("POST", "/api/pairing/revoke"),
    ] {
        let (code, body) = client.request(method, path, &[], None).unwrap();
        assert_eq!(code, 401, "{method} {path}: {:?}", body);
    }
    assert!(
        client.ping().unwrap(),
        "denied shutdown must have no effect"
    );
    assert!(server.pairing().list_devices().is_empty());
}

#[test]
fn pairing_requires_host_initiation_and_devices_never_list_tokens() {
    let (_tmp, server) = server();
    let host = DaemonClient::from_uds(server.socket_path());
    let client = DaemonClient::from_http(server.http_addr().unwrap());
    let (code, body) = host
        .request("GET", "/api/pairing/initiate", &[], None)
        .unwrap();
    assert_eq!(code, 200);
    let session: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        session["endpoint_url"],
        format!("http://{}", server.http_addr().unwrap())
    );
    let payload = serde_json::to_vec(&json!({
        "pairing_token": session["pairing_token"], "device_name": "fixture"
    }))
    .unwrap();
    let (code, body) = client
        .request("POST", "/api/pairing/complete", &[], Some(&payload))
        .unwrap();
    assert_eq!(code, 200);
    let device: Value = serde_json::from_slice(&body).unwrap();
    let token = device["auth_token"].as_str().unwrap();
    let authorization = format!("Bearer {token}");
    let headers = [("Authorization", authorization.as_str())];
    let (code, _) = client
        .request("GET", "/interactions", &headers, None)
        .unwrap();
    assert_eq!(code, 200);
    for (method, path) in [
        ("POST", "/shutdown"),
        ("POST", "/lifecycle/mode"),
        ("GET", "/api/pairing/initiate"),
        ("GET", "/api/pairing/devices"),
        ("POST", "/api/pairing/revoke"),
    ] {
        assert_eq!(client.request(method, path, &headers, None).unwrap().0, 403);
    }
    let (_, body) = host
        .request("GET", "/api/pairing/devices", &[], None)
        .unwrap();
    let listed: Value = serde_json::from_slice(&body).unwrap();
    assert!(listed[0].get("auth_token").is_none());
    assert!(!String::from_utf8(body).unwrap().contains(token));
    assert_eq!(
        client
            .request("POST", "/api/pairing/complete", &[], Some(&payload))
            .unwrap()
            .0,
        400
    );
    assert!(server
        .pairing()
        .revoke_device(device["device_id"].as_str().unwrap()));
    assert_eq!(
        client
            .request("GET", "/interactions", &headers, None)
            .unwrap()
            .0,
        401
    );
}

#[test]
fn browser_origin_and_ambiguous_or_oversized_headers_are_rejected() {
    let (_tmp, server) = server();
    for method in ["GET", "POST", "OPTIONS"] {
        let request =
            format!("{method} /ping HTTP/1.1\r\nOrigin: https://untrusted.example\r\n\r\n");
        let response = raw_request(&server, request.as_bytes());
        assert!(
            response.starts_with("HTTP/1.1 403"),
            "{method}: {response:?}"
        );
        assert!(!response.contains("Access-Control-Allow-Origin"));
    }
    for headers in [
        "Content-Length: invalid\r\n",
        "Content-Length: 0\r\nContent-Length: 2\r\n",
        "Transfer-Encoding: chunked\r\n",
        "not-a-header\r\n",
    ] {
        let request = format!("GET /ping HTTP/1.1\r\n{headers}\r\n");
        assert!(raw_request(&server, request.as_bytes()).starts_with("HTTP/1.1 400"));
    }
    for request in [
        format!("GET /{} HTTP/1.1\r\n\r\n", "x".repeat(17 * 1024)),
        format!(
            "GET /ping HTTP/1.1\r\nX-Large: {}\r\n\r\n",
            "x".repeat(17 * 1024)
        ),
    ] {
        assert!(raw_request(&server, request.as_bytes()).starts_with("HTTP/1.1 431"));
    }
    assert!(raw_request(
        &server,
        b"POST /api/pairing/complete HTTP/1.1\r\nContent-Length: 1048577\r\n\r\n"
    )
    .starts_with("HTTP/1.1 413"));
}

#[test]
fn plaintext_http_cannot_bind_to_all_network_interfaces() {
    let tmp = tempdir().unwrap();
    let config = DaemonConfig::new()
        .with_socket_path(tmp.path().join("engine.sock"))
        .with_db_path(tmp.path().join("ledger.db"))
        .with_http_addr(Some("0.0.0.0:0".into()));
    assert!(DaemonServer::start(config).is_err());
    assert!(!tmp.path().join("engine.sock").exists());
    assert!(!tmp.path().join("ledger.db").exists());
}

#[test]
fn socket_cleanup_preserves_regular_files_and_symlinks() {
    let tmp = tempdir().unwrap();
    let socket = tmp.path().join("engine.sock");
    let config = DaemonConfig::new()
        .with_socket_path(&socket)
        .with_http_addr(None)
        .with_db_path(tmp.path().join("ledger.db"));
    std::fs::write(&socket, b"user file").unwrap();
    assert!(DaemonServer::start(config.clone()).is_err());
    assert_eq!(std::fs::read(&socket).unwrap(), b"user file");
    std::fs::remove_file(&socket).unwrap();
    let target = tmp.path().join("outside");
    std::fs::write(&target, b"preserve target").unwrap();
    std::os::unix::fs::symlink(&target, &socket).unwrap();
    assert!(DaemonServer::start(config.clone()).is_err());
    assert!(std::fs::symlink_metadata(&socket)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(std::fs::read(&target).unwrap(), b"preserve target");
    std::fs::remove_file(&socket).unwrap();
    std::os::unix::fs::symlink(tmp.path().join("missing"), &socket).unwrap();
    assert!(DaemonServer::start(config).is_err());
    assert!(std::fs::symlink_metadata(&socket)
        .unwrap()
        .file_type()
        .is_symlink());
}

fn wait_for_subscriber(server: &DaemonServer) {
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while server.broadcaster().subscriber_count() == 0 {
        assert!(std::time::Instant::now() < deadline, "SSE must subscribe");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn sse_live_events_respect_session_filter() {
    let (_tmp, server) = server();
    let host = DaemonClient::from_uds(server.socket_path());
    let mut events = host.open_events_stream(None, Some(1)).unwrap();
    wait_for_subscriber(&server);
    host.publish_event("Msg", "other session", Some(2), Some("fixture"))
        .unwrap();
    host.publish_event("Msg", "requested session", Some(1), Some("fixture"))
        .unwrap();
    assert_eq!(
        events.next_event().unwrap().unwrap().body,
        "requested session"
    );
}

#[test]
fn revoking_device_closes_its_existing_sse_stream() {
    let (_tmp, server) = server();
    let session = server
        .pairing()
        .initiate_session(server.http_addr().unwrap())
        .unwrap();
    let device = server
        .pairing()
        .complete_pairing(&session.pairing_token, "fixture")
        .unwrap();
    let mut stream = TcpStream::connect(server.http_addr().unwrap()).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    write!(
        stream,
        "GET /events HTTP/1.1\r\nAuthorization: Bearer {}\r\n\r\n",
        device.auth_token
    )
    .unwrap();
    let mut reader = BufReader::new(stream);
    let mut status = String::new();
    reader.read_line(&mut status).unwrap();
    assert!(status.starts_with("HTTP/1.1 200"));
    loop {
        let mut line = String::new();
        assert!(reader.read_line(&mut line).unwrap() > 0);
        if line.trim().is_empty() {
            break;
        }
    }
    wait_for_subscriber(&server);
    assert!(server.pairing().revoke_device(&device.device_id));
    server
        .broadcaster()
        .broadcast(chorus_daemon::events::DaemonEvent::new(
            1,
            0,
            None,
            "Msg",
            "fixture",
            "must not reach revoked device",
        ));
    let mut body = String::new();
    reader.read_to_string(&mut body).unwrap();
    assert!(body.is_empty());
}
