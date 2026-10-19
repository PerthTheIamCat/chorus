//! Stopping a process: the rules (pure, unit-tested) and the flow.
//!
//! Research 03 §4 "Killing processes":
//! 1. Signal only a pid that passes every check at click time *and again just
//!    before `kill`*: our uid, unchanged start time and path (pid reuse), not
//!    under an Apple system path, not pid ≤ 1, not Chorus or its daemon.
//! 2. The UI confirms first. SIGTERM, wait, then SIGKILL only when the user
//!    clicks "บังคับปิด" — never automatically. `killpg` only for groups
//!    Chorus created (the pid registry), never groups inferred from parents.
//! 3. Labels are hints; nothing here trusts argv or the process name.
//! 5. A typed request; Swift only confirms. No MCP tool exposes this.

use serde::{Deserialize, Serialize};

/// Who a row was when it was drawn. The stop request carries this back so a
/// reused pid is never signalled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ident {
    pub pid: i32,
    pub start_sec: u64,
    pub start_usec: u64,
    pub path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Signal {
    Term,
    Kill,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StopRequest {
    pub ident: Ident,
    pub signal: Signal,
    /// Signal the whole process group the pid leads. Allowed only when the
    /// pid is a group leader Chorus spawned (see [`Facts::chorus_group`]).
    #[serde(default)]
    pub group: bool,
}

/// Why a stop was refused. Shown to the user as-is (Thai copy in `message`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Refusal {
    Gone,
    PidReused,
    OtherUser,
    SystemPid,
    ChorusItself,
    SystemPath,
    GroupNotOurs,
}

impl Refusal {
    pub fn message(self) -> &'static str {
        match self {
            Refusal::Gone => "process นี้ปิดไปแล้ว",
            Refusal::PidReused => "pid นี้เป็นของ process ใหม่แล้ว (ตัวเดิมปิดไป) — ไม่ส่งสัญญาณ",
            Refusal::OtherUser => "เป็น process ของผู้ใช้อื่น",
            Refusal::SystemPid => "เป็น process ของระบบ",
            Refusal::ChorusItself => "เป็นตัว Chorus เอง — ใช้ Quit Chorus แทน",
            Refusal::SystemPath => "อยู่ใน path ของระบบ macOS (launchd จะเปิดกลับมาเองอยู่ดี)",
            Refusal::GroupNotOurs => "ปิดทั้งกลุ่มได้เฉพาะกลุ่มที่ Chorus สร้างเอง",
        }
    }
}

/// What the kernel says about the pid right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Facts {
    pub pid: i32,
    pub uid: u32,
    pub start_sec: u64,
    pub start_usec: u64,
    pub path: String,
    pub zombie: bool,
    /// `pid` is a registered group leader Chorus spawned, still the same
    /// process (registry spawn time matches the kernel start time).
    pub chorus_group: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Context {
    pub my_uid: u32,
    pub my_pid: i32,
    /// Pids of Chorus's own daemon (`chorusd`) and anything else that must
    /// go through "Quit Chorus" instead.
    pub protected: Vec<i32>,
}

/// Apple system locations. A uid check is not enough: ControlCenter (:5000,
/// :7000) and rapportd run as the user and own listening ports.
pub fn is_system_path(path: &str) -> bool {
    const DENY: &[&str] = &[
        "/System/",
        "/bin/",
        "/sbin/",
        "/usr/",
        "/Library/Apple/",
        "/private/var/db/",
        "/Applications/Safari.app/",
        "/Applications/Xcode.app/Contents/MacOS/",
    ];
    const ALLOW_UNDER_USR: &[&str] = &["/usr/local/"];
    if path.is_empty() {
        // An unreadable path is not a reason to trust the process.
        return true;
    }
    if ALLOW_UNDER_USR.iter().any(|p| path.starts_with(p)) {
        return false;
    }
    DENY.iter().any(|p| path.starts_with(p))
}

/// The rule set. `facts` is `None` when the pid no longer exists.
pub fn check(req: &StopRequest, facts: Option<&Facts>, ctx: &Context) -> Result<(), Refusal> {
    let pid = req.ident.pid;
    if pid <= 1 {
        return Err(Refusal::SystemPid);
    }
    if pid == ctx.my_pid || ctx.protected.contains(&pid) {
        return Err(Refusal::ChorusItself);
    }
    let Some(f) = facts else {
        return Err(Refusal::Gone);
    };
    if f.zombie {
        return Err(Refusal::Gone);
    }
    if f.uid != ctx.my_uid {
        return Err(Refusal::OtherUser);
    }
    if f.start_sec != req.ident.start_sec
        || f.start_usec != req.ident.start_usec
        || f.path != req.ident.path
    {
        return Err(Refusal::PidReused);
    }
    if is_system_path(&f.path) {
        return Err(Refusal::SystemPath);
    }
    if req.group && !f.chorus_group {
        return Err(Refusal::GroupNotOurs);
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StopState {
    /// Signal sent; the process has not exited yet.
    Signalled,
    /// The process is gone (or a zombie).
    Exited,
    Refused,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct StopOutcome {
    pub state: StopState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refusal: Option<Refusal>,
    pub message: String,
}

impl StopOutcome {
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))] // only the macOS stop path refuses
    fn refused(r: Refusal) -> Self {
        StopOutcome {
            state: StopState::Refused,
            refusal: Some(r),
            message: r.message().into(),
        }
    }
}

#[cfg(target_os = "macos")]
fn current_facts(pid: i32) -> Option<Facts> {
    use super::darwin;
    let (uid, start_sec, start_usec, zombie) = darwin::identity(pid)?;
    let path = darwin::path_of(pid);
    let chorus_group = super::registered_leader(pid, start_sec).is_some();
    Some(Facts {
        pid,
        uid,
        start_sec,
        start_usec,
        path,
        zombie,
        chorus_group,
    })
}

#[cfg(target_os = "macos")]
pub fn context() -> Context {
    Context {
        my_uid: super::darwin::my_uid(),
        my_pid: super::darwin::my_pid(),
        protected: super::daemon_pids(),
    }
}

/// Re-check, then send one signal. Never escalates on its own.
#[cfg(target_os = "macos")]
pub fn stop(req: &StopRequest) -> StopOutcome {
    let ctx = context();
    if let Err(r) = check(req, current_facts(req.ident.pid).as_ref(), &ctx) {
        return StopOutcome::refused(r);
    }
    let sig = match req.signal {
        Signal::Term => libc::SIGTERM,
        Signal::Kill => libc::SIGKILL,
    };
    match super::darwin::signal(req.ident.pid, sig, req.group) {
        Ok(()) => StopOutcome {
            state: StopState::Signalled,
            refusal: None,
            message: "ส่งสัญญาณแล้ว".into(),
        },
        Err(libc::ESRCH) => StopOutcome::refused(Refusal::Gone),
        Err(e) => StopOutcome {
            state: StopState::Failed,
            refusal: None,
            message: format!("ส่งสัญญาณไม่สำเร็จ: {}", std::io::Error::from_raw_os_error(e)),
        },
    }
}

/// Poll every 100 ms for up to `timeout_ms` until the process is gone.
/// A zombie counts as gone (and is reaped if it is our own child).
#[cfg(target_os = "macos")]
pub fn wait_exit(ident: &Ident, timeout_ms: u64) -> StopOutcome {
    let deadline =
        std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms.min(10_000));
    loop {
        let alive = match super::darwin::identity(ident.pid) {
            None => false,
            Some((_, s, us, zombie)) => {
                if zombie {
                    super::darwin::reap(ident.pid);
                }
                !zombie && s == ident.start_sec && us == ident.start_usec
            }
        };
        if !alive {
            return StopOutcome {
                state: StopState::Exited,
                refusal: None,
                message: "ปิดแล้ว".into(),
            };
        }
        if std::time::Instant::now() >= deadline {
            return StopOutcome {
                state: StopState::Signalled,
                refusal: None,
                message: "ยังไม่ปิด — กด \"บังคับปิด\" ถ้าต้องการส่ง SIGKILL".into(),
            };
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> Context {
        Context {
            my_uid: 501,
            my_pid: 4000,
            protected: vec![4001],
        }
    }

    fn facts() -> Facts {
        Facts {
            pid: 7000,
            uid: 501,
            start_sec: 1_790_000_000,
            start_usec: 42,
            path: "/opt/homebrew/bin/redis-server".into(),
            zombie: false,
            chorus_group: false,
        }
    }

    fn req(f: &Facts) -> StopRequest {
        StopRequest {
            ident: Ident {
                pid: f.pid,
                start_sec: f.start_sec,
                start_usec: f.start_usec,
                path: f.path.clone(),
            },
            signal: Signal::Term,
            group: false,
        }
    }

    #[test]
    fn own_dev_server_may_be_stopped() {
        let f = facts();
        assert_eq!(check(&req(&f), Some(&f), &ctx()), Ok(()));
    }

    #[test]
    fn other_users_processes_are_refused() {
        let mut f = facts();
        f.uid = 0;
        assert_eq!(check(&req(&f), Some(&f), &ctx()), Err(Refusal::OtherUser));
    }

    #[test]
    fn reused_pid_is_refused() {
        let f = facts();
        let r = req(&f);
        let mut now = f.clone();
        now.start_sec += 5;
        assert_eq!(check(&r, Some(&now), &ctx()), Err(Refusal::PidReused));
        let mut now = f.clone();
        now.start_usec += 1;
        assert_eq!(check(&r, Some(&now), &ctx()), Err(Refusal::PidReused));
        let mut now = f;
        now.path = "/Users/me/evil".into();
        assert_eq!(check(&r, Some(&now), &ctx()), Err(Refusal::PidReused));
    }

    #[test]
    fn gone_and_zombie_are_refused() {
        let f = facts();
        assert_eq!(check(&req(&f), None, &ctx()), Err(Refusal::Gone));
        let mut z = f.clone();
        z.zombie = true;
        assert_eq!(check(&req(&f), Some(&z), &ctx()), Err(Refusal::Gone));
    }

    #[test]
    fn launchd_and_chorus_are_refused() {
        let mut f = facts();
        f.pid = 1;
        assert_eq!(check(&req(&f), Some(&f), &ctx()), Err(Refusal::SystemPid));
        f.pid = 0;
        assert_eq!(check(&req(&f), Some(&f), &ctx()), Err(Refusal::SystemPid));
        f.pid = -1;
        assert_eq!(check(&req(&f), Some(&f), &ctx()), Err(Refusal::SystemPid));
        f.pid = 4000;
        assert_eq!(
            check(&req(&f), Some(&f), &ctx()),
            Err(Refusal::ChorusItself)
        );
        f.pid = 4001;
        assert_eq!(
            check(&req(&f), Some(&f), &ctx()),
            Err(Refusal::ChorusItself)
        );
    }

    #[test]
    fn system_paths_are_refused_even_for_our_uid() {
        for p in [
            "/System/Library/CoreServices/ControlCenter.app/Contents/MacOS/ControlCenter",
            "/usr/libexec/rapportd",
            "/usr/bin/python3",
            "/bin/sh",
            "/sbin/launchd",
            "/Library/Apple/System/Library/x",
            "",
        ] {
            let mut f = facts();
            f.path = p.into();
            assert_eq!(
                check(&req(&f), Some(&f), &ctx()),
                Err(Refusal::SystemPath),
                "{p}"
            );
        }
        for p in ["/usr/local/bin/redis-server", "/opt/homebrew/bin/postgres", "/Users/me/.nvm/versions/node/v22/bin/node",
                  "/Library/Developer/CommandLineTools/Library/Frameworks/Python3.framework/Versions/3.9/Resources/Python.app/Contents/MacOS/Python"] {
            assert!(!is_system_path(p), "{p}");
        }
    }

    #[test]
    fn killpg_only_for_groups_chorus_created() {
        let f = facts();
        let mut r = req(&f);
        r.group = true;
        assert_eq!(check(&r, Some(&f), &ctx()), Err(Refusal::GroupNotOurs));
        let mut ours = f;
        ours.chorus_group = true;
        assert_eq!(check(&r, Some(&ours), &ctx()), Ok(()));
    }

    #[test]
    fn kill_is_checked_exactly_like_term() {
        let mut f = facts();
        f.uid = 0;
        let mut r = req(&f);
        r.signal = Signal::Kill;
        assert_eq!(check(&r, Some(&f), &ctx()), Err(Refusal::OtherUser));
    }
}
