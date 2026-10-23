//! A real pseudo-terminal for the app's integrated terminal.
//!
//! All terminal *logic* lives here (CLAUDE.md 2.3): spawning the shell,
//! environment, resize, kill. The Swift shell only draws the grid and
//! forwards keystrokes.
//!
//! No new dependency: `openpty`/`ioctl`/`setsid` are declared by hand — they
//! are in libSystem on macOS and in glibc ≥ 2.34 on Linux. The shell is
//! spawned with `std::process::Command`; its three stdio fds are the slave
//! side, `pre_exec` makes it a session leader with the slave as controlling
//! terminal (without that `^C`, job control and `tty` do not work).
//!
//! The shell is the user's own shell — it is *not* sandboxed by default (the
//! user typing is not an agent). `PtyOptions::sandbox` wraps it in the same
//! Seatbelt profile agents get, for a terminal that should behave like one.

use std::ffi::{c_int, c_ulong, c_void};
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use crate::sandbox::{self, Profile};

#[repr(C)]
struct Winsize {
    ws_row: u16,
    ws_col: u16,
    ws_xpixel: u16,
    ws_ypixel: u16,
}

extern "C" {
    fn openpty(
        amaster: *mut c_int,
        aslave: *mut c_int,
        name: *mut c_void,
        termp: *const c_void,
        winp: *const Winsize,
    ) -> c_int;
    fn ioctl(fd: c_int, req: c_ulong, ...) -> c_int;
    fn setsid() -> c_int;
    fn kill(pid: c_int, sig: c_int) -> c_int;
}

#[cfg(target_os = "macos")]
const TIOCSWINSZ: c_ulong = 0x8008_7467;
#[cfg(target_os = "macos")]
const TIOCSCTTY: c_ulong = 0x2000_7461;
#[cfg(not(target_os = "macos"))]
const TIOCSWINSZ: c_ulong = 0x5414;
#[cfg(not(target_os = "macos"))]
const TIOCSCTTY: c_ulong = 0x540E;

const SIGHUP: c_int = 1;
const SIGKILL: c_int = 9;

#[derive(Debug, Clone, Default)]
pub struct PtyOptions {
    pub cwd: PathBuf,
    pub cols: u16,
    pub rows: u16,
    /// Defaults to `$SHELL`, then `/bin/zsh`.
    pub shell: Option<String>,
    /// Confine writes like an agent's turn. `None` = the user's own shell.
    pub sandbox: Option<Profile>,
    /// Extra environment (e.g. Chorus-injected secrets, project vars).
    pub env: Vec<(String, String)>,
}

pub struct Pty {
    writer: Mutex<File>,
    master: File,
    pid: i32,
    /// Why the shell is not confined when `sandbox` was requested.
    pub sandbox_note: Option<String>,
}

fn winsize(cols: u16, rows: u16) -> Winsize {
    Winsize {
        ws_row: rows.max(1),
        ws_col: cols.max(1),
        ws_xpixel: 0,
        ws_ypixel: 0,
    }
}

/// The environment a terminal gets: the repaired PATH (a Finder launch has
/// launchd's bare one), a colour-capable `TERM`, and a UTF-8 locale — without
/// one Thai input echoes as `?` in zsh line editing.
fn base_env() -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = vec![
        ("TERM".into(), "xterm-256color".into()),
        ("COLORTERM".into(), "truecolor".into()),
        ("TERM_PROGRAM".into(), "Chorus".into()),
        ("CHORUS_TERMINAL".into(), "1".into()),
        (
            "PATH".into(),
            crate::env::agent_path(None).to_string_lossy().into_owned(),
        ),
    ];
    let utf8 = |k: &str| std::env::var(k).is_ok_and(|v| v.to_uppercase().contains("UTF-8"));
    if !utf8("LC_ALL") && !utf8("LANG") {
        env.push(("LANG".into(), "en_US.UTF-8".into()));
    }
    env
}

/// Where a sandboxed shell keeps what it would normally write under `$HOME`.
/// Seatbelt only lets it write caches and temp, so zsh's history (and its lock
/// file), `.zcompdump` and oh-my-zsh's cache would otherwise fail on every
/// start with "operation not permitted". `~/.cache` is on the allow list
/// (CLAUDE.md 7.20), so history still survives between terminals.
fn sandbox_shell_env(home: &Path) -> Vec<(String, String)> {
    let dir = home.join(".cache").join("chorus").join("term");
    let _ = std::fs::create_dir_all(&dir);
    let at = |name: &str| dir.join(name).to_string_lossy().into_owned();
    vec![
        ("HISTFILE".into(), at("zsh_history")),
        ("ZSH_COMPDUMP".into(), at("zcompdump")),
        ("ZSH_CACHE_DIR".into(), dir.to_string_lossy().into_owned()),
    ]
}

impl Pty {
    /// Spawn the shell. Returns the PTY and a reader thread handle: it calls
    /// `on_data` for every chunk, then `on_exit(code)` once (code is -1 when the
    /// shell died to a signal).
    pub fn spawn(
        opts: PtyOptions,
        on_data: impl Fn(&[u8]) + Send + 'static,
        on_exit: impl FnOnce(i32) + Send + 'static,
    ) -> Result<Arc<Pty>, String> {
        if !opts.cwd.is_dir() {
            return Err(format!("ไม่พบโฟลเดอร์ {}", opts.cwd.display()));
        }
        let (mut m, mut s) = (-1, -1);
        let ws = winsize(opts.cols, opts.rows);
        // SAFETY: out-pointers are valid; name/termp may be null.
        let rc = unsafe { openpty(&mut m, &mut s, std::ptr::null_mut(), std::ptr::null(), &ws) };
        if rc != 0 {
            return Err(format!("openpty: {}", std::io::Error::last_os_error()));
        }
        // SAFETY: openpty returned two fresh fds that we now own.
        let (master, slave) = unsafe { (OwnedFd::from_raw_fd(m), OwnedFd::from_raw_fd(s)) };

        let shell = opts
            .shell
            .clone()
            .or_else(|| std::env::var("SHELL").ok().filter(|s| !s.is_empty()))
            .unwrap_or_else(|| "/bin/zsh".into());
        let mut argv = vec![shell, "-l".to_string(), "-i".to_string()];
        let mut sandbox_note = None;
        if let Some(profile) = opts.sandbox {
            match sandbox::wrap(&argv, &opts.cwd, profile) {
                Ok(w) => argv = w,
                Err(why) => sandbox_note = Some(why),
            }
        }

        let mut cmd = Command::new(&argv[0]);
        cmd.args(&argv[1..]).current_dir(&opts.cwd).env_remove("CLAUDECODE");
        // Explicit options win, then the sandbox redirects, then the base env.
        let mut env = base_env();
        if opts.sandbox.is_some() && sandbox_note.is_none() {
            if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
                env.extend(sandbox_shell_env(&home));
            }
        }
        for (k, v) in env.into_iter().chain(opts.env) {
            cmd.env(k, v);
        }
        let slave_fd = slave.as_raw_fd();
        let slave_out = slave.try_clone().map_err(|e| e.to_string())?;
        let slave_err = slave.try_clone().map_err(|e| e.to_string())?;
        cmd.stdin(Stdio::from(slave)).stdout(Stdio::from(slave_out)).stderr(Stdio::from(slave_err));
        // SAFETY: only async-signal-safe calls (setsid, ioctl) between fork and exec.
        unsafe {
            cmd.pre_exec(move || {
                setsid();
                ioctl(slave_fd, TIOCSCTTY, 0 as c_int);
                Ok(())
            });
        }
        let mut child = cmd.spawn().map_err(|e| format!("เปิดเชลล์ไม่ได้: {e}"))?;
        let pid = child.id() as i32;
        // The parent's copies of the slave were moved into `cmd` and drop with
        // it, so the master sees EOF when the shell exits.
        drop(cmd);

        let master = File::from(master);
        let mut reader = master.try_clone().map_err(|e| e.to_string())?;
        let writer = master.try_clone().map_err(|e| e.to_string())?;
        let pty = Arc::new(Pty { writer: Mutex::new(writer), master, pid, sandbox_note });

        std::thread::Builder::new()
            .name("chorus-pty".into())
            .spawn(move || {
                let mut buf = [0u8; 16 * 1024];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => on_data(&buf[..n]),
                        // EIO is how a pty master says the slave closed.
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(_) => break,
                    }
                }
                let code = child.wait().ok().and_then(|s| s.code()).unwrap_or(-1);
                on_exit(code);
            })
            .map_err(|e| e.to_string())?;
        Ok(pty)
    }

    pub fn write(&self, bytes: &[u8]) -> std::io::Result<()> {
        self.writer.lock().unwrap().write_all(bytes)
    }

    pub fn resize(&self, cols: u16, rows: u16) {
        let ws = winsize(cols, rows);
        // SAFETY: valid fd and a properly laid out winsize.
        unsafe { ioctl(self.master.as_raw_fd(), TIOCSWINSZ, &ws as *const Winsize) };
    }

    /// Hang up the shell; if it ignores that, the reader thread's `wait` is
    /// still pending, so follow with SIGKILL.
    pub fn close(&self) {
        // SAFETY: plain signal to a pid we spawned.
        unsafe {
            kill(self.pid, SIGHUP);
            kill(self.pid, SIGKILL);
        }
    }

    pub fn pid(&self) -> i32 {
        self.pid
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    fn run(input: &str, wait_for: &str) -> String {
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        let dir = tempfile_dir();
        let pty = Pty::spawn(
            PtyOptions {
                cwd: dir,
                cols: 80,
                rows: 24,
                shell: Some("/bin/sh".into()),
                ..Default::default()
            },
            move |b| {
                let _ = tx.send(b.to_vec());
            },
            |_| {},
        )
        .unwrap();
        pty.write(input.as_bytes()).unwrap();
        let mut got = String::new();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while std::time::Instant::now() < deadline && !got.contains(wait_for) {
            if let Ok(b) = rx.recv_timeout(Duration::from_millis(200)) {
                got.push_str(&String::from_utf8_lossy(&b));
            }
        }
        got
    }

    #[test]
    fn sandboxed_shell_writes_go_under_the_allowed_cache_dir() {
        let home = tempfile_dir().join("home-sbx");
        let env = sandbox_shell_env(&home);
        let get = |k: &str| env.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()).unwrap();
        for k in ["HISTFILE", "ZSH_COMPDUMP", "ZSH_CACHE_DIR"] {
            assert!(get(k).starts_with(home.join(".cache").to_str().unwrap()), "{k}");
        }
        assert!(home.join(".cache/chorus/term").is_dir());
    }

    fn tempfile_dir() -> PathBuf {
        std::env::temp_dir()
    }

    #[test]
    fn shell_runs_commands_on_a_real_tty() {
        // `tty` prints /dev/ttysNNN only when stdin is a terminal.
        let out = run("tty; echo done-$((20+22))\n", "done-42");
        assert!(out.contains("/dev/tty"), "not a tty: {out:?}");
        assert!(out.contains("done-42"), "{out:?}");
    }

    #[test]
    fn thai_round_trips_as_utf8() {
        let out = run("echo สวัสดี-ok\n", "สวัสดี-ok");
        assert!(out.contains("สวัสดี-ok"), "{out:?}");
    }

    #[test]
    fn resize_is_seen_by_the_shell() {
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        let pty = Pty::spawn(
            PtyOptions { cwd: tempfile_dir(), cols: 80, rows: 24, shell: Some("/bin/sh".into()), ..Default::default() },
            move |b| {
                let _ = tx.send(b.to_vec());
            },
            |_| {},
        )
        .unwrap();
        pty.resize(120, 40);
        pty.write(b"stty size\n").unwrap();
        let mut got = String::new();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while std::time::Instant::now() < deadline && !got.contains("40 120") {
            if let Ok(b) = rx.recv_timeout(Duration::from_millis(200)) {
                got.push_str(&String::from_utf8_lossy(&b));
            }
        }
        assert!(got.contains("40 120"), "{got:?}");
    }

    #[test]
    fn exit_is_reported_with_the_code() {
        let (tx, rx) = mpsc::channel::<i32>();
        let pty = Pty::spawn(
            PtyOptions { cwd: tempfile_dir(), cols: 80, rows: 24, shell: Some("/bin/sh".into()), ..Default::default() },
            |_| {},
            move |c| {
                let _ = tx.send(c);
            },
        )
        .unwrap();
        pty.write(b"exit 7\n").unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(10)).unwrap(), 7);
    }

    #[test]
    fn missing_cwd_is_an_error_not_a_hang() {
        let r = Pty::spawn(
            PtyOptions { cwd: "/nonexistent/chorus".into(), cols: 80, rows: 24, ..Default::default() },
            |_| {},
            |_| {},
        );
        assert!(r.is_err());
    }
}
