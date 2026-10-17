//! System monitor: which ports and services are running, who started them,
//! what they cost, and Chorus's own RAM and disk (ticket 14, research 02).
//!
//! The API is platform-neutral rows; only `darwin.rs` touches libproc, so a
//! Windows backend later replaces that file and nothing else.
//!
//! It never touches SQLite or the engine handle on the snapshot path, so it
//! runs on its own thread and is not subject to the single-queue rule (7.7).
//! Disk and optimise actions open their own connections on demand.

#[cfg(target_os = "macos")]
mod darwin;
pub mod disk;
pub mod optimise;
pub mod service;
pub mod stop;

use serde::{Deserialize, Serialize};
use service::ServiceName;
use std::collections::HashMap;
use stop::{Ident, Refusal};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// This process: the Chorus app (or whatever hosts the core).
    App,
    /// `chorusd`.
    Daemon,
    /// An agent bridge Chorus spawned (from the pid registry).
    Bridge,
    /// Started by a bridge or by Chorus (same group or parent chain).
    Child,
    /// The local model runtime (Ollama, llama.cpp).
    Model,
}

/// One process, as the window draws it.
#[derive(Debug, Clone, Serialize)]
pub struct Row {
    pub pid: i32,
    pub ppid: i32,
    pub name: String,
    pub path: String,
    /// Masked argv (token-like values replaced). Empty for other users'
    /// processes. Never the environment.
    pub cmdline: Vec<String>,
    pub cpu_pct: Option<f64>,
    /// `ri_phys_footprint` in bytes — Activity Monitor's "Memory".
    pub footprint: Option<u64>,
    pub uptime_s: Option<u64>,
    /// Owned by our uid. Other users' rows are drawn greyed with no actions.
    pub mine: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<Role>,
    /// "agent codex-acp", "Chorus" … A hint, not proof (research 02 §3).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_by: Option<String>,
    /// Identity to send back with a stop request. `None` = no actions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ident: Option<Ident>,
    /// Why the stop button is not offered, if it is not.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refusal: Option<Refusal>,
    /// `refusal` in words, for the row's tooltip.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refusal_note: Option<String>,
    /// This row leads a process group Chorus created: "stop everything this
    /// agent started" may use `killpg` on it.
    pub group_stoppable: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ServiceRow {
    pub port: u16,
    pub udp: bool,
    /// Local addresses bound (`127.0.0.1`, `*`, `::1`).
    pub addrs: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service: Option<ServiceName>,
    pub process: Row,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Snapshot {
    pub chorus: Vec<Row>,
    pub services: Vec<ServiceRow>,
    /// Footprint of the app, daemon, bridges and their children (not the
    /// local model, which is listed separately because it is shared).
    pub chorus_footprint: u64,
    pub chorus_cpu_pct: f64,
    pub model_footprint: u64,
    pub sampled_ms: u64,
    /// How long the sweep took, for the canary and for the window's footer.
    pub sweep_ms: f64,
    /// False on platforms without a backend.
    pub supported: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SnapshotOptions {
    /// Include bound UDP sockets (ephemeral and port-0 ones are dropped).
    #[serde(default)]
    pub udp: bool,
    /// Skip the services sweep: the sidebar label needs only the Chorus group.
    #[serde(default)]
    pub chorus_only: bool,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Registry entry for `pid`, if it is still the process we spawned: the
/// kernel's start time must fall just before our recorded spawn time.
pub fn registered_leader(pid: i32, start_sec: u64) -> Option<chorus_acp::registry::Spawned> {
    chorus_acp::registry::list().into_iter().find(|s| {
        let spawned_s = s.spawned_ms / 1000;
        s.pid as i32 == pid
            && s.group_leader
            && start_sec <= spawned_s + 1
            && start_sec + 10 >= spawned_s
    })
}

#[cfg(not(target_os = "macos"))]
pub fn snapshot(_opts: &SnapshotOptions) -> Snapshot {
    Snapshot {
        sampled_ms: now_ms(),
        ..Default::default()
    }
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn daemon_pids() -> Vec<i32> {
    Vec::new()
}

#[cfg(target_os = "macos")]
pub(crate) fn daemon_pids() -> Vec<i32> {
    let me = darwin::my_pid();
    darwin::own_pids()
        .into_iter()
        .filter(|&p| p != me)
        .filter(|&p| darwin::path_of(p).rsplit('/').next() == Some("chorusd"))
        .collect()
}

type Key = (i32, u64, u64);
/// `(port, pid, local addr)` of a listener another user owns.
type Foreign = (u16, i32, String);

#[derive(Default)]
struct State {
    /// Previous CPU sample per process, for CPU% = Δcpu / Δwall.
    cpu: HashMap<Key, (u64, std::time::Instant)>,
    /// Masked argv and the service name derived from the raw argv, per
    /// process lifetime. Raw argv is never stored.
    argv: HashMap<Key, (Vec<String>, Option<ServiceName>)>,
    netstat: Option<(std::time::Instant, Vec<Foreign>)>,
}

static STATE: std::sync::Mutex<Option<State>> = std::sync::Mutex::new(None);

#[cfg(target_os = "macos")]
const MODEL_EXES: &[&str] = &["ollama", "llama-server", "ollama_llama_server"];

#[cfg(target_os = "macos")]
pub fn snapshot(opts: &SnapshotOptions) -> Snapshot {
    use darwin::RawProc;
    let t0 = std::time::Instant::now();
    let my_uid = darwin::my_uid();
    let my_pid = darwin::my_pid();
    let ctx = stop::context();

    let mut procs: HashMap<i32, RawProc> = darwin::own_pids()
        .into_iter()
        .filter_map(|p| darwin::read_proc(p).map(|r| (p, r)))
        .collect();
    if let Some(me) = darwin::read_proc(my_pid) {
        procs.entry(my_pid).or_insert(me);
    }

    // Registered bridges that are still the process we spawned.
    let bridges: HashMap<i32, String> = chorus_acp::registry::list()
        .into_iter()
        .filter_map(|s| {
            let p = procs.get(&(s.pid as i32))?;
            registered_leader(p.pid, p.start_sec).map(|_| (p.pid, s.label))
        })
        .collect();

    let descends_from = |mut pid: i32, root: i32| -> bool {
        for _ in 0..64 {
            if pid == root {
                return true;
            }
            match procs.get(&pid) {
                Some(p) if p.ppid > 1 && p.ppid != pid => pid = p.ppid,
                _ => return false,
            }
        }
        false
    };

    let started_by = |p: &RawProc| -> Option<String> {
        if let Some(l) = bridges.get(&p.pid) {
            return Some(format!("agent {l}"));
        }
        if let Some(l) = bridges.get(&p.pgid) {
            return Some(format!("agent {l}"));
        }
        if let Some((_, l)) = bridges.iter().find(|(&b, _)| descends_from(p.pid, b)) {
            return Some(format!("agent {l}"));
        }
        (p.pid != my_pid && descends_from(p.pid, my_pid)).then(|| "Chorus".to_string())
    };

    let role_of = |p: &RawProc| -> Option<Role> {
        let exe = p.path.rsplit('/').next().unwrap_or("");
        if p.pid == my_pid {
            Some(Role::App)
        } else if exe == "chorusd" {
            Some(Role::Daemon)
        } else if bridges.contains_key(&p.pid) {
            Some(Role::Bridge)
        } else if MODEL_EXES.contains(&exe) || MODEL_EXES.contains(&p.comm.as_str()) {
            Some(Role::Model)
        } else if started_by(p).is_some() {
            Some(Role::Child)
        } else {
            None
        }
    };

    // Sockets for our own processes.
    let mut sockets: Vec<darwin::RawSocket> = Vec::new();
    if !opts.chorus_only {
        for &pid in procs.keys() {
            sockets.extend(
                darwin::sockets(pid, opts.udp)
                    .into_iter()
                    .filter(|s| keep_socket(s.udp, s.port)),
            );
        }
    }

    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let state = guard.get_or_insert_with(State::default);
    let now = std::time::Instant::now();
    let wall_s = now_ms() / 1000;

    let mut seen: Vec<Key> = Vec::new();
    let mut this_sweep: HashMap<Key, Option<f64>> = HashMap::new();
    let mut make_row = |p: &RawProc,
                        state: &mut State,
                        port: Option<(u16, bool)>|
     -> (Row, Option<ServiceName>) {
        let mine = p.uid == my_uid;
        let key = (p.pid, p.start_sec, p.start_usec);
        let mut p = p.clone();
        let mut cpu_pct = None;
        if mine {
            darwin::fill_usage(&mut p);
            // A process can be drawn twice in one sweep (Chorus group and a
            // port it owns); sample its CPU once, or the second read sees a
            // zero-length interval.
            if let Some(&pct) = this_sweep.get(&key) {
                cpu_pct = pct;
            } else if let Some(ns) = p.cpu_ns {
                if let Some((prev, at)) = state.cpu.get(&key) {
                    let dt = now.duration_since(*at).as_nanos() as f64;
                    if dt > 0.0 && ns >= *prev {
                        cpu_pct = Some(((ns - prev) as f64 / dt * 100.0 * 10.0).round() / 10.0);
                    }
                }
                state.cpu.insert(key, (ns, now));
                this_sweep.insert(key, cpu_pct);
            }
            seen.push(key);
        }
        let (cmdline, svc) = if mine {
            state
                .argv
                .entry(key)
                .or_insert_with(|| {
                    let mut raw = darwin::argv(p.pid);
                    // Processes that rewrite their title (redis) leave empty slots.
                    raw.retain(|a| !a.is_empty());
                    let svc = service::name_service(
                        &p.path,
                        &p.comm,
                        &raw,
                        port.map(|x| x.0),
                        port.is_some_and(|x| x.1),
                    );
                    (service::mask_args(&raw), svc)
                })
                .clone()
        } else {
            (
                Vec::new(),
                service::name_service(
                    &p.path,
                    &p.comm,
                    &[],
                    port.map(|x| x.0),
                    port.is_some_and(|x| x.1),
                ),
            )
        };
        // A port-only guess depends on the port, so recompute it per socket.
        let svc = match (svc, port) {
            (Some(s), _) if s.source != service::NameSource::Port => Some(s),
            (_, Some((port, udp))) => service::name_service(&p.path, &p.comm, &[], Some(port), udp),
            (s, None) => s,
        };
        let ident = mine.then(|| Ident {
            pid: p.pid,
            start_sec: p.start_sec,
            start_usec: p.start_usec,
            path: p.path.clone(),
        });
        let refusal = match &ident {
            Some(id) => {
                let req = stop::StopRequest {
                    ident: id.clone(),
                    signal: stop::Signal::Term,
                    group: false,
                };
                let facts = stop::Facts {
                    pid: p.pid,
                    uid: p.uid,
                    start_sec: p.start_sec,
                    start_usec: p.start_usec,
                    path: p.path.clone(),
                    zombie: p.zombie,
                    chorus_group: false,
                };
                stop::check(&req, Some(&facts), &ctx).err()
            }
            None => Some(Refusal::OtherUser),
        };
        let row = Row {
            pid: p.pid,
            ppid: p.ppid,
            name: p.comm.clone(),
            path: p.path.clone(),
            cmdline,
            cpu_pct,
            footprint: p.footprint,
            uptime_s: (p.start_sec > 0).then(|| wall_s.saturating_sub(p.start_sec)),
            mine,
            role: role_of(&p),
            started_by: started_by(&p),
            refusal,
            refusal_note: refusal.map(|r| r.message().to_string()),
            group_stoppable: mine && bridges.contains_key(&p.pid) && p.pgid == p.pid,
            ident: ident.filter(|_| refusal.is_none()),
        };
        (row, svc)
    };

    // Chorus group.
    let mut chorus: Vec<Row> = procs
        .values()
        .filter(|p| role_of(p).is_some())
        .map(|p| make_row(p, state, None).0)
        .collect();
    chorus.sort_by_key(|r| (r.role.map(|x| x as u8).unwrap_or(9), r.pid));
    let chorus_footprint = chorus
        .iter()
        .filter(|r| r.role != Some(Role::Model))
        .filter_map(|r| r.footprint)
        .sum::<u64>();
    let chorus_cpu_pct = chorus
        .iter()
        .filter(|r| r.role != Some(Role::Model))
        .filter_map(|r| r.cpu_pct)
        .sum::<f64>();
    let model_footprint = chorus
        .iter()
        .filter(|r| r.role == Some(Role::Model))
        .filter_map(|r| r.footprint)
        .sum();

    // Services group: one row per (pid, port, proto), addresses merged.
    let mut grouped: Vec<((u16, bool, i32), Vec<String>)> = Vec::new();
    for s in &sockets {
        match grouped
            .iter_mut()
            .find(|(k, _)| *k == (s.port, s.udp, s.pid))
        {
            Some((_, a)) if !a.contains(&s.addr) => a.push(s.addr.clone()),
            Some(_) => {}
            None => grouped.push(((s.port, s.udp, s.pid), vec![s.addr.clone()])),
        }
    }
    let mut services: Vec<ServiceRow> = grouped
        .into_iter()
        .filter_map(|((port, udp, pid), addrs)| {
            let p = procs.get(&pid)?;
            let (process, service) = make_row(p, state, Some((port, udp)));
            Some(ServiceRow {
                port,
                udp,
                addrs,
                service,
                process,
            })
        })
        .collect();

    // Other users' TCP listeners: only the kernel table knows them. Greyed,
    // no actions; refreshed at most every 5 s (it is a subprocess).
    if !opts.chorus_only {
        let fresh = state
            .netstat
            .as_ref()
            .is_some_and(|(at, _)| now.duration_since(*at).as_secs() < 5);
        if !fresh {
            state.netstat = Some((now, netstat_listeners()));
        }
        let foreign: Vec<Foreign> = state
            .netstat
            .as_ref()
            .map(|(_, v)| v.clone())
            .unwrap_or_default();
        for (port, pid, addr) in foreign {
            if procs.contains_key(&pid)
                || services
                    .iter()
                    .any(|s| s.port == port && s.process.pid == pid && !s.udp)
            {
                continue;
            }
            let Some(p) = darwin::read_proc(pid) else {
                continue;
            };
            if p.uid == my_uid {
                continue; // ours, already swept (or exited between the two reads)
            }
            if let Some(s) = services
                .iter_mut()
                .find(|s| s.port == port && s.process.pid == pid)
            {
                if !s.addrs.contains(&addr) {
                    s.addrs.push(addr);
                }
                continue;
            }
            let (process, service) = make_row(&p, state, Some((port, false)));
            services.push(ServiceRow {
                port,
                udp: false,
                addrs: vec![addr],
                service,
                process,
            });
        }
    }
    services.sort_by(|a, b| (a.port, a.udp, a.process.pid).cmp(&(b.port, b.udp, b.process.pid)));

    // Forget processes that are gone so the caches do not grow forever.
    if !opts.chorus_only {
        state.cpu.retain(|k, _| seen.contains(k));
        state.argv.retain(|k, _| seen.contains(k));
    }

    Snapshot {
        chorus,
        services,
        chorus_footprint,
        chorus_cpu_pct: (chorus_cpu_pct * 10.0).round() / 10.0,
        model_footprint,
        sampled_ms: now_ms(),
        sweep_ms: (t0.elapsed().as_secs_f64() * 1000.0 * 10.0).round() / 10.0,
        supported: true,
    }
}

/// One typed request from the shell (`chorus_monitor_json`). Swift builds
/// these; every decision is made here.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    Snapshot(SnapshotOptions),
    Stop(stop::StopRequest),
    WaitExit {
        ident: Ident,
        #[serde(default = "default_wait")]
        timeout_ms: u64,
    },
    Disk {
        workspaces: Vec<String>,
    },
    Preview {
        action: optimise::Action,
    },
    Apply {
        action: optimise::Action,
    },
}

fn default_wait() -> u64 {
    3000
}

/// Run a request. `protect` = checkpoint shas the caller still needs (the
/// open session's base); they are added to any checkpoint prune.
pub fn handle(req: Request, protect: &[String]) -> Result<serde_json::Value, String> {
    let with_protect = |mut a: optimise::Action| {
        if let optimise::Action::PruneCheckpoints { protect: p, .. } = &mut a {
            p.extend(protect.iter().filter(|s| !s.is_empty()).cloned());
        }
        a
    };
    let v = match req {
        Request::Snapshot(o) => serde_json::to_value(snapshot(&o)),
        #[cfg(target_os = "macos")]
        Request::Stop(r) => serde_json::to_value(stop::stop(&r)),
        #[cfg(target_os = "macos")]
        Request::WaitExit { ident, timeout_ms } => {
            serde_json::to_value(stop::wait_exit(&ident, timeout_ms))
        }
        #[cfg(not(target_os = "macos"))]
        Request::Stop(_) | Request::WaitExit { .. } => {
            return Err("not supported on this platform".into())
        }
        Request::Disk { workspaces } => serde_json::to_value(disk::report(&workspaces)),
        Request::Preview { action } => {
            serde_json::to_value(optimise::preview(&with_protect(action))?)
        }
        Request::Apply { action } => serde_json::to_value(optimise::apply(&with_protect(action))?),
    };
    v.map_err(|e| e.to_string())
}

/// UDP is noisy: drop port 0 and the ephemeral range (49152–65535) — those
/// are clients, not services.
fn keep_socket(udp: bool, port: u16) -> bool {
    port != 0 && !(udp && port >= 49152)
}

/// `netstat -anv -p tcp` LISTEN rows as `(port, pid, addr)`. Best effort:
/// the `process:pid` column is not documented, so any parse miss is skipped.
#[cfg(target_os = "macos")]
fn netstat_listeners() -> Vec<Foreign> {
    let Ok(out) = std::process::Command::new("/usr/sbin/netstat")
        .args(["-anv", "-p", "tcp"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
    else {
        return Vec::new();
    };
    parse_netstat(&String::from_utf8_lossy(&out.stdout))
}

pub(crate) fn parse_netstat(text: &str) -> Vec<Foreign> {
    let mut out = Vec::new();
    for line in text.lines() {
        let t: Vec<&str> = line.split_whitespace().collect();
        if t.len() < 12 || !t[0].starts_with("tcp") || t[5] != "LISTEN" {
            continue;
        }
        let Some((addr, port)) = t[3].rsplit_once('.') else {
            continue;
        };
        let Ok(port) = port.parse::<u16>() else {
            continue;
        };
        // `process:pid` may contain spaces ("Claude Helper:72925"); it ends
        // at the token before the 5-hex-digit `state` column.
        let Some(end) =
            (10..t.len()).find(|&i| t[i].len() == 5 && t[i].chars().all(|c| c.is_ascii_hexdigit()))
        else {
            continue;
        };
        let proc_pid = t[10..end].join(" ");
        let Some(pid) = proc_pid
            .rsplit_once(':')
            .and_then(|(_, p)| p.parse::<i32>().ok())
        else {
            continue;
        };
        let addr = if addr == "*" {
            "*".to_string()
        } else {
            addr.to_string()
        };
        if !out.contains(&(port, pid, addr.clone())) {
            out.push((port, pid, addr));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn netstat_rows_with_spaces_in_the_name() {
        let text = "\
Proto Recv-Q Send-Q  Local Address          Foreign Address        (state)          rxbytes      txbytes  rhiwat  shiwat          process:pid    state  options           gencnt    flags   flags1 usecnt rtncnt fltrs
tcp4       0      0  127.0.0.1.20241        *.*                    LISTEN                 0            0  131072  131072  cloudflared:34997  00100 00000006 000000000007852f 00000001 00000800      1      0 000000
tcp6       0      0  *.65482                *.*                    LISTEN                 0            0  131072  131072  Claude Helper:72925  00100 00000006 00000000000a0219 00000001 00000800      1      0 000000
tcp4       0      0  10.0.0.2.54673         1.2.3.4.443            ESTABLISHED        11390         1910  131072  132104             node:16276  00102 00000108 00000000000ceac1 00000081 04000900      2      0 000000
";
        assert_eq!(
            parse_netstat(text),
            vec![
                (20241, 34997, "127.0.0.1".into()),
                (65482, 72925, "*".into())
            ]
        );
    }

    #[test]
    fn udp_noise_is_dropped() {
        assert!(!keep_socket(true, 0));
        assert!(!keep_socket(true, 55000));
        assert!(keep_socket(true, 5353));
        assert!(
            keep_socket(false, 55000),
            "a TCP listener on a high port is still a service"
        );
    }
}
