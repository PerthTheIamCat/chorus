//! Canary for the libproc layer (Apple calls it private and "subject to
//! change"): if a macOS update moves a struct field or flips byte order, the
//! listener this test opens stops showing up. Plus the stop flow end to end,
//! against processes this test spawned itself — nothing else is signalled.
#![cfg(target_os = "macos")]

use chorus_daemon::monitor::{self, stop, Request, SnapshotOptions};
use std::net::{TcpListener, UdpSocket};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command};
use std::time::Duration;

fn snap(udp: bool) -> monitor::Snapshot {
    monitor::snapshot(&SnapshotOptions {
        udp,
        chorus_only: false,
    })
}

#[test]
fn canary_finds_our_own_listener() {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let s = snap(false);
    let me = std::process::id() as i32;
    let row = s
        .services
        .iter()
        .find(|r| r.port == port && !r.udp)
        .unwrap_or_else(|| {
            panic!(
                "listener on {port} not found; libproc layout changed? sweep={}ms",
                s.sweep_ms
            )
        });
    assert_eq!(row.process.pid, me);
    assert_eq!(row.addrs, vec!["127.0.0.1".to_string()]);
    assert!(row.process.mine);
    assert!(
        row.process.footprint.unwrap_or(0) > 1024 * 1024,
        "footprint {:?}",
        row.process.footprint
    );
    // We are the app here, so the guard must refuse to stop ourselves.
    assert_eq!(row.process.refusal, Some(stop::Refusal::ChorusItself));
    assert!(row.process.ident.is_none());
    assert!(s.sweep_ms < 500.0, "sweep took {}ms", s.sweep_ms);
    drop(l);
}

#[test]
fn canary_ipv6_and_udp() {
    let l6 = TcpListener::bind("[::1]:0").unwrap();
    let p6 = l6.local_addr().unwrap().port();
    let u = UdpSocket::bind("127.0.0.1:0").unwrap();
    let pu = u.local_addr().unwrap().port();
    let s = snap(true);
    assert!(
        s.services
            .iter()
            .any(|r| r.port == p6 && r.addrs.contains(&"::1".to_string())),
        "ipv6 listener"
    );
    // Ephemeral UDP ports are noise and are dropped on purpose.
    let found_udp = s.services.iter().any(|r| r.port == pu && r.udp);
    assert_eq!(found_udp, pu < 49152);
    assert!(
        !snap(false).services.iter().any(|r| r.udp),
        "UDP only behind the toggle"
    );
}

#[test]
fn cpu_percent_needs_two_samples_then_reports() {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let _ = snap(false);
    // Burn a little CPU so the delta is non-zero.
    let t = std::time::Instant::now();
    let mut x = 0u64;
    while t.elapsed() < Duration::from_millis(150) {
        x = x.wrapping_mul(6364136223846793005).wrapping_add(1);
    }
    std::hint::black_box(x);
    let s = snap(false);
    let row = s.services.iter().find(|r| r.port == port).unwrap();
    assert!(
        row.process.cpu_pct.unwrap_or(0.0) > 0.0,
        "cpu {:?}",
        row.process.cpu_pct
    );
}

/// A child process we own and may signal: this test binary, re-run in
/// helper mode. (A copy of `/bin/sleep` would dodge the system-path guard,
/// but macOS SIGKILLs Apple platform binaries run from elsewhere.)
fn helper(ignore_term: bool, group: bool) -> Child {
    let mut cmd = Command::new(std::env::current_exe().unwrap());
    cmd.args([
        "--ignored",
        "--exact",
        "helper_child",
        "--test-threads=1",
        "-q",
    ])
    .env("CHORUS_MONITOR_HELPER", "1");
    if group {
        cmd.process_group(0);
    }
    if ignore_term {
        // An ignored disposition survives exec.
        // SAFETY: async-signal-safe call between fork and exec.
        unsafe {
            cmd.pre_exec(|| {
                libc::signal(libc::SIGTERM, libc::SIG_IGN);
                Ok(())
            });
        }
    }
    cmd.stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    cmd.spawn().unwrap()
}

#[test]
#[ignore = "child process for the stop-flow tests"]
fn helper_child() {
    if std::env::var_os("CHORUS_MONITOR_HELPER").is_some() {
        std::thread::sleep(Duration::from_secs(30));
    }
}

fn row_for(pid: u32) -> monitor::Row {
    for _ in 0..50 {
        let s = monitor::snapshot(&SnapshotOptions {
            udp: false,
            chorus_only: true,
        });
        if let Some(r) = s.chorus.into_iter().find(|r| r.pid == pid as i32) {
            return r;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("child {pid} not listed in the Chorus group");
}

fn stop_req(row: &monitor::Row, signal: stop::Signal, group: bool) -> Request {
    Request::Stop(stop::StopRequest {
        ident: row.ident.clone().expect("stoppable"),
        signal,
        group,
    })
}

fn state(v: &serde_json::Value) -> &str {
    v["state"].as_str().unwrap_or("")
}

fn reap(mut c: Child) {
    let _ = c.wait();
}

#[test]
fn term_stops_a_well_behaved_child() {
    let child = helper(false, false);
    let row = row_for(child.id());
    assert_eq!(
        row.started_by.as_deref(),
        Some("Chorus"),
        "parent chain → Chorus"
    );
    let r = monitor::handle(stop_req(&row, stop::Signal::Term, false), &[]).unwrap();
    assert_eq!(state(&r), "signalled", "{r}");
    let w = monitor::handle(
        Request::WaitExit {
            ident: row.ident.clone().unwrap(),
            timeout_ms: 3000,
        },
        &[],
    )
    .unwrap();
    assert_eq!(state(&w), "exited", "{w}");
    reap(child);
    // The pid is gone now: a second request must be refused, not sent.
    let again = monitor::handle(stop_req(&row, stop::Signal::Kill, false), &[]).unwrap();
    assert_eq!(state(&again), "refused", "{again}");
}

#[test]
fn sigkill_needs_its_own_request() {
    // Shrugs off SIGTERM.
    let child = helper(true, false);
    let row = row_for(child.id());
    let r = monitor::handle(stop_req(&row, stop::Signal::Term, false), &[]).unwrap();
    assert_eq!(state(&r), "signalled");
    let w = monitor::handle(
        Request::WaitExit {
            ident: row.ident.clone().unwrap(),
            timeout_ms: 600,
        },
        &[],
    )
    .unwrap();
    assert_eq!(
        state(&w),
        "signalled",
        "TERM ignored: still alive, and nothing escalated on its own: {w}"
    );
    let k = monitor::handle(stop_req(&row, stop::Signal::Kill, false), &[]).unwrap();
    assert_eq!(state(&k), "signalled");
    let w = monitor::handle(
        Request::WaitExit {
            ident: row.ident.clone().unwrap(),
            timeout_ms: 3000,
        },
        &[],
    )
    .unwrap();
    assert_eq!(state(&w), "exited", "{w}");
    reap(child);
}

#[test]
fn reused_identity_is_never_signalled() {
    let child = helper(false, false);
    let row = row_for(child.id());
    let mut ident = row.ident.clone().unwrap();
    ident.start_usec = ident.start_usec.wrapping_add(1); // as if the pid were reused
    let r = monitor::handle(
        Request::Stop(stop::StopRequest {
            ident,
            signal: stop::Signal::Kill,
            group: false,
        }),
        &[],
    )
    .unwrap();
    assert_eq!(r["refusal"], "pid_reused", "{r}");
    // Still running: clean up through the real identity.
    let r = monitor::handle(stop_req(&row, stop::Signal::Kill, false), &[]).unwrap();
    assert_eq!(state(&r), "signalled");
    reap(child);
}

#[test]
fn killpg_only_for_registered_groups() {
    let child = helper(false, true);
    let row = row_for(child.id());
    assert!(!row.group_stoppable);
    let r = monitor::handle(stop_req(&row, stop::Signal::Term, true), &[]).unwrap();
    assert_eq!(r["refusal"], "group_not_ours", "{r}");

    chorus_acp::registry::register(child.id(), "test-acp", true);
    let row = row_for(child.id());
    assert!(row.group_stoppable, "{row:?}");
    assert_eq!(row.started_by.as_deref(), Some("agent test-acp"));
    let r = monitor::handle(stop_req(&row, stop::Signal::Term, true), &[]).unwrap();
    assert_eq!(state(&r), "signalled", "{r}");
    let w = monitor::handle(
        Request::WaitExit {
            ident: row.ident.clone().unwrap(),
            timeout_ms: 3000,
        },
        &[],
    )
    .unwrap();
    assert_eq!(state(&w), "exited");
    chorus_acp::registry::unregister(child.id());
    reap(child);
}

#[test]
fn requests_parse_from_the_shells_json() {
    let r: Request = serde_json::from_str(r#"{"op":"snapshot","udp":true}"#).unwrap();
    assert!(matches!(
        r,
        Request::Snapshot(SnapshotOptions { udp: true, .. })
    ));
    let r: Request = serde_json::from_str(
        r#"{"op":"preview","action":{"action":"prune_checkpoints","workspace":"/x","days":30}}"#,
    )
    .unwrap();
    assert!(matches!(r, Request::Preview { .. }));
    let r: Request = serde_json::from_str(r#"{"op":"stop","ident":{"pid":5,"start_sec":1,"start_usec":2,"path":"/p"},"signal":"term"}"#).unwrap();
    assert!(matches!(
        r,
        Request::Stop(stop::StopRequest { group: false, .. })
    ));
}
