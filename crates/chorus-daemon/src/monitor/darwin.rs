//! libproc through `libc`: the only platform-specific part of the monitor.
//!
//! Apple's header calls these "private interfaces … subject to change". They
//! have shipped since 10.5 and `lsof`/`top` use them, but the canary test in
//! `tests/monitor.rs` binds a listener and checks this finds it — that is what
//! notices a struct-layout or byte-order change on a new macOS.
//!
//! `libc` binds the process calls but not the socket structs, so those are
//! mirrored by hand from `sys/proc_info.h` (MacOSX27 SDK) with size asserts.

use std::ffi::{c_int, c_void, CStr};
use std::mem::{size_of, zeroed, MaybeUninit};

/// Everything one sweep reads about a process. Fields we could not read
/// (another user's process: `EPERM`) are `None`.
#[derive(Debug, Clone, Default)]
pub struct RawProc {
    pub pid: i32,
    pub ppid: i32,
    pub pgid: i32,
    pub uid: u32,
    pub zombie: bool,
    /// Kernel start time; with `pid` it identifies one process for its life.
    pub start_sec: u64,
    pub start_usec: u64,
    pub comm: String,
    pub path: String,
    /// user + system CPU time in nanoseconds (already converted from ticks).
    pub cpu_ns: Option<u64>,
    /// `ri_phys_footprint`: the number Activity Monitor and `top` show.
    pub footprint: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawSocket {
    pub pid: i32,
    pub udp: bool,
    pub port: u16,
    /// Local address as text: `127.0.0.1`, `::1`, `*`.
    pub addr: String,
}

// ---- hand-mirrored socket structs (sys/proc_info.h) ---------------------

const PROC_PIDFDSOCKETINFO: c_int = 3;
const PROC_UID_ONLY: u32 = 4;
const SOCKINFO_IN: c_int = 1;
const SOCKINFO_TCP: c_int = 2;
const TSI_S_LISTEN: c_int = 1;
const INI_IPV4: u8 = 0x1;
const INI_IPV6: u8 = 0x2;
const IPPROTO_UDP: c_int = 17;

#[repr(C)]
struct ProcFileInfo {
    fi_openflags: u32,
    fi_status: u32,
    fi_offset: i64,
    fi_type: i32,
    fi_guardflags: u32,
}

#[repr(C)]
struct SockbufInfo {
    sbi_cc: u32,
    sbi_hiwat: u32,
    sbi_mbcnt: u32,
    sbi_mbmax: u32,
    sbi_lowat: u32,
    sbi_flags: i16,
    sbi_timeo: i16,
}

#[repr(C)]
struct InSockInfo {
    insi_fport: c_int,
    /// Network byte order in the low 16 bits.
    insi_lport: c_int,
    insi_gencnt: u64,
    insi_flags: u32,
    insi_flow: u32,
    insi_vflag: u8,
    insi_ip_ttl: u8,
    rfu_1: u32,
    insi_faddr: [u8; 16],
    /// `in4in6_addr` (12 pad bytes, then the IPv4 address) or `in6_addr`.
    insi_laddr: [u8; 16],
    insi_v4: u8,
    insi_v6: [u8; 12],
}

#[repr(C)]
struct TcpSockInfo {
    tcpsi_ini: InSockInfo,
    tcpsi_state: c_int,
    tcpsi_timer: [c_int; 4],
    tcpsi_mss: c_int,
    tcpsi_flags: u32,
    rfu_1: u32,
    tcpsi_tp: u64,
}

#[repr(C)]
struct SocketInfo {
    /// `struct vinfo_stat`, unused here.
    soi_stat: [u64; 17],
    soi_so: u64,
    soi_pcb: u64,
    soi_type: c_int,
    soi_protocol: c_int,
    soi_family: c_int,
    soi_options: i16,
    soi_linger: i16,
    soi_state: i16,
    soi_qlen: i16,
    soi_incqlen: i16,
    soi_qlimit: i16,
    soi_timeo: i16,
    soi_error: u16,
    soi_oobmark: u32,
    soi_rcv: SockbufInfo,
    soi_snd: SockbufInfo,
    soi_kind: c_int,
    rfu_1: u32,
    /// The union; its largest member is `un_sockinfo` (528 bytes).
    soi_proto: [u64; 66],
}

#[repr(C)]
struct SocketFdInfo {
    pfi: ProcFileInfo,
    psi: SocketInfo,
}

// Measured with the SDK's own headers (`sizeof`, `offsetof`) on macOS 27.
const _: () = assert!(size_of::<ProcFileInfo>() == 24);
const _: () = assert!(size_of::<InSockInfo>() == 80);
const _: () = assert!(size_of::<TcpSockInfo>() == 120);
const _: () = assert!(size_of::<SocketInfo>() == 768);
const _: () = assert!(size_of::<SocketFdInfo>() == 792);
const _: () = assert!(std::mem::offset_of!(SocketInfo, soi_kind) == 232);
const _: () = assert!(std::mem::offset_of!(SocketInfo, soi_proto) == 240);
const _: () = assert!(std::mem::offset_of!(InSockInfo, insi_laddr) == 48);

// ---- timebase -----------------------------------------------------------

/// `pti_total_user/system` are mach ticks (125/3 ns each on Apple Silicon,
/// 1 ns on Intel), not nanoseconds.
fn ticks_to_ns(ticks: u64) -> u64 {
    // libc deprecates its binding in favour of the `mach2` crate; the call is
    // one struct and one function, so declare it here instead of a new dep.
    #[repr(C)]
    struct Timebase {
        numer: u32,
        denom: u32,
    }
    extern "C" {
        fn mach_timebase_info(info: *mut Timebase) -> c_int;
    }
    static TB: std::sync::OnceLock<(u32, u32)> = std::sync::OnceLock::new();
    let (numer, denom) = *TB.get_or_init(|| {
        let mut info = Timebase { numer: 0, denom: 0 };
        // SAFETY: plain out-pointer to a stack struct.
        let r = unsafe { mach_timebase_info(&mut info) };
        if r != 0 || info.denom == 0 {
            (1, 1)
        } else {
            (info.numer, info.denom)
        }
    });
    ((ticks as u128 * numer as u128) / denom as u128) as u64
}

// ---- process reads ------------------------------------------------------

pub fn my_uid() -> u32 {
    // SAFETY: no arguments, cannot fail.
    unsafe { libc::getuid() }
}

pub fn my_pid() -> i32 {
    std::process::id() as i32
}

fn c_name(buf: &[libc::c_char]) -> String {
    let bytes: Vec<u8> = buf
        .iter()
        .take_while(|&&c| c != 0)
        .map(|&c| c as u8)
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Pids owned by our uid (cheaper than all pids: no `EPERM` calls).
pub fn own_pids() -> Vec<i32> {
    list_pids(Some(my_uid()))
}

fn list_pids(uid: Option<u32>) -> Vec<i32> {
    let mut cap = 4096usize;
    loop {
        let mut buf = vec![0i32; cap];
        let bytes = (cap * size_of::<i32>()) as c_int;
        // SAFETY: buffer sized to `bytes`.
        let n = unsafe {
            match uid {
                Some(u) => {
                    libc::proc_listpids(PROC_UID_ONLY, u, buf.as_mut_ptr() as *mut c_void, bytes)
                }
                None => libc::proc_listallpids(buf.as_mut_ptr() as *mut c_void, bytes),
            }
        };
        if n <= 0 {
            return Vec::new();
        }
        // proc_listpids returns bytes; proc_listallpids returns a count.
        let count = if uid.is_some() {
            n as usize / size_of::<i32>()
        } else {
            n as usize
        };
        if count >= cap {
            cap *= 2;
            continue;
        }
        buf.truncate(count);
        buf.retain(|&p| p > 0);
        return buf;
    }
}

pub fn path_of(pid: i32) -> String {
    let mut buf = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    // SAFETY: buffer sized as the header requires.
    let n = unsafe { libc::proc_pidpath(pid, buf.as_mut_ptr() as *mut c_void, buf.len() as u32) };
    if n <= 0 {
        return String::new();
    }
    buf.truncate(n as usize);
    String::from_utf8_lossy(&buf).into_owned()
}

fn bsd_info(pid: i32) -> Option<libc::proc_bsdinfo> {
    let mut info = MaybeUninit::<libc::proc_bsdinfo>::zeroed();
    let size = size_of::<libc::proc_bsdinfo>() as c_int;
    // SAFETY: buffer sized to the struct.
    let n = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            info.as_mut_ptr() as *mut c_void,
            size,
        )
    };
    // SAFETY: zeroed is a valid bit pattern; the kernel filled it on success.
    (n == size).then(|| unsafe { info.assume_init() })
}

fn short_info(pid: i32) -> Option<libc::proc_bsdshortinfo> {
    let mut info = MaybeUninit::<libc::proc_bsdshortinfo>::zeroed();
    let size = size_of::<libc::proc_bsdshortinfo>() as c_int;
    // SAFETY: as above.
    let n = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDT_SHORTBSDINFO,
            0,
            info.as_mut_ptr() as *mut c_void,
            size,
        )
    };
    (n == size).then(|| unsafe { info.assume_init() })
}

fn cpu_ns(pid: i32) -> Option<u64> {
    let mut info = MaybeUninit::<libc::proc_taskinfo>::zeroed();
    let size = size_of::<libc::proc_taskinfo>() as c_int;
    // SAFETY: as above.
    let n = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTASKINFO,
            0,
            info.as_mut_ptr() as *mut c_void,
            size,
        )
    };
    if n != size {
        return None;
    }
    let t = unsafe { info.assume_init() };
    Some(ticks_to_ns(
        t.pti_total_user.saturating_add(t.pti_total_system),
    ))
}

fn footprint(pid: i32) -> Option<u64> {
    let mut info = MaybeUninit::<libc::rusage_info_v4>::zeroed();
    // SAFETY: flavor V4 writes exactly a rusage_info_v4.
    let r = unsafe {
        libc::proc_pid_rusage(
            pid,
            libc::RUSAGE_INFO_V4,
            info.as_mut_ptr() as *mut libc::rusage_info_t,
        )
    };
    (r == 0).then(|| unsafe { info.assume_init() }.ri_phys_footprint)
}

/// Read one process. `None` when it no longer exists.
///
/// Our own processes give the full BSD info (with start time); other users'
/// only give the short form, so their start time is 0 — they are never
/// signalled, so they never need an identity.
pub fn read_proc(pid: i32) -> Option<RawProc> {
    if let Some(b) = bsd_info(pid) {
        return Some(RawProc {
            pid,
            ppid: b.pbi_ppid as i32,
            pgid: b.pbi_pgid as i32,
            uid: b.pbi_uid,
            zombie: b.pbi_status == libc::SZOMB,
            start_sec: b.pbi_start_tvsec,
            start_usec: b.pbi_start_tvusec,
            comm: {
                let name = c_name(&b.pbi_name);
                if name.is_empty() {
                    c_name(&b.pbi_comm)
                } else {
                    name
                }
            },
            path: path_of(pid),
            cpu_ns: None,
            footprint: None,
        });
    }
    let s = short_info(pid)?;
    Some(RawProc {
        pid,
        ppid: s.pbsi_ppid as i32,
        pgid: s.pbsi_pgid as i32,
        uid: s.pbsi_uid,
        zombie: s.pbsi_status == libc::SZOMB,
        comm: c_name(&s.pbsi_comm),
        path: path_of(pid),
        ..Default::default()
    })
}

/// CPU time and footprint, read only for rows the monitor shows.
pub fn fill_usage(p: &mut RawProc) {
    p.cpu_ns = cpu_ns(p.pid);
    p.footprint = footprint(p.pid);
}

/// Cheap existence check for the stop flow: identity fields only.
pub fn identity(pid: i32) -> Option<(u32, u64, u64, bool)> {
    bsd_info(pid).map(|b| {
        (
            b.pbi_uid,
            b.pbi_start_tvsec,
            b.pbi_start_tvusec,
            b.pbi_status == libc::SZOMB,
        )
    })
}

/// Listening TCP sockets (and bound UDP ones when `udp`) owned by `pid`.
/// Other users' processes return nothing (`EPERM` on the fd list).
pub fn sockets(pid: i32, udp: bool) -> Vec<RawSocket> {
    let fds = list_fds(pid);
    let mut out = Vec::new();
    for fd in fds {
        if fd.proc_fdtype != libc::PROX_FDTYPE_SOCKET as u32 {
            continue;
        }
        // SAFETY: zeroed is valid for this plain-data struct.
        let mut si: SocketFdInfo = unsafe { zeroed() };
        let size = size_of::<SocketFdInfo>() as c_int;
        // SAFETY: buffer is exactly PROC_PIDFDSOCKETINFO_SIZE.
        let n = unsafe {
            libc::proc_pidfdinfo(
                pid,
                fd.proc_fd,
                PROC_PIDFDSOCKETINFO,
                &mut si as *mut _ as *mut c_void,
                size,
            )
        };
        if n != size {
            continue;
        }
        let kind = si.psi.soi_kind;
        let proto = si.psi.soi_proto.as_ptr() as *const u8;
        let (ini, is_udp) = if kind == SOCKINFO_TCP {
            // SAFETY: soi_kind says the union holds a tcp_sockinfo.
            let tcp = unsafe { &*(proto as *const TcpSockInfo) };
            if tcp.tcpsi_state != TSI_S_LISTEN {
                continue;
            }
            (&tcp.tcpsi_ini, false)
        } else if kind == SOCKINFO_IN && udp && si.psi.soi_protocol == IPPROTO_UDP {
            // SAFETY: soi_kind says the union holds an in_sockinfo.
            (unsafe { &*(proto as *const InSockInfo) }, true)
        } else {
            continue;
        };
        let port = u16::from_be(ini.insi_lport as u16);
        if port == 0 {
            continue;
        }
        out.push(RawSocket {
            pid,
            udp: is_udp,
            port,
            addr: local_addr(ini),
        });
    }
    out.sort_by(|a, b| (a.port, a.udp, &a.addr).cmp(&(b.port, b.udp, &b.addr)));
    out.dedup();
    out
}

fn local_addr(ini: &InSockInfo) -> String {
    let a = &ini.insi_laddr;
    if ini.insi_vflag & INI_IPV4 != 0 {
        let v4 = std::net::Ipv4Addr::new(a[12], a[13], a[14], a[15]);
        if v4.is_unspecified() {
            "*".into()
        } else {
            v4.to_string()
        }
    } else if ini.insi_vflag & INI_IPV6 != 0 {
        let v6 = std::net::Ipv6Addr::from(*a);
        if v6.is_unspecified() {
            "*".into()
        } else {
            v6.to_string()
        }
    } else {
        "*".into()
    }
}

fn list_fds(pid: i32) -> Vec<libc::proc_fdinfo> {
    let mut cap = 256usize;
    loop {
        // SAFETY: zeroed is valid for proc_fdinfo.
        let mut buf: Vec<libc::proc_fdinfo> = vec![unsafe { zeroed() }; cap];
        let bytes = (cap * size_of::<libc::proc_fdinfo>()) as c_int;
        // SAFETY: buffer sized to `bytes`.
        let n = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDLISTFDS,
                0,
                buf.as_mut_ptr() as *mut c_void,
                bytes,
            )
        };
        if n <= 0 {
            return Vec::new();
        }
        let count = n as usize / size_of::<libc::proc_fdinfo>();
        if count >= cap && cap < 1 << 16 {
            cap *= 4;
            continue;
        }
        buf.truncate(count);
        return buf;
    }
}

/// argv of one of our own processes, from `KERN_PROCARGS2`.
///
/// That buffer also holds the process's **environment**, right after argv.
/// We stop at `argc` strings and wipe the buffer before it is dropped, so no
/// environment byte outlives this function (CLAUDE.md §11, 7.18).
pub fn argv(pid: i32) -> Vec<String> {
    let mut size: usize = 0;
    let mut mib = [libc::CTL_KERN, libc::KERN_ARGMAX];
    let mut argmax: c_int = 0;
    let mut len = size_of::<c_int>();
    // SAFETY: reads one int.
    if unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            2,
            &mut argmax as *mut _ as *mut c_void,
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    } == 0
    {
        size = argmax.max(0) as usize;
    }
    if size == 0 {
        return Vec::new();
    }
    let mut buf = vec![0u8; size];
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
    // SAFETY: buffer of `size` bytes, `size` updated with the bytes written.
    let r = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            buf.as_mut_ptr() as *mut c_void,
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    let out = if r == 0 {
        parse_procargs(&buf[..size.min(buf.len())])
    } else {
        Vec::new()
    };
    wipe(&mut buf);
    out
}

fn wipe(buf: &mut [u8]) {
    for b in buf.iter_mut() {
        // SAFETY: valid &mut u8; volatile so the wipe is not optimised away.
        unsafe { std::ptr::write_volatile(b, 0) };
    }
}

/// `KERN_PROCARGS2` layout: `int argc`, exec path, NUL padding, then argc
/// NUL-terminated strings, then the environment (never read).
pub(crate) fn parse_procargs(buf: &[u8]) -> Vec<String> {
    if buf.len() < 4 {
        return Vec::new();
    }
    let argc = i32::from_ne_bytes([buf[0], buf[1], buf[2], buf[3]]).clamp(0, 4096) as usize;
    let mut i = 4;
    while i < buf.len() && buf[i] != 0 {
        i += 1; // exec path
    }
    while i < buf.len() && buf[i] == 0 {
        i += 1; // padding
    }
    let mut args = Vec::with_capacity(argc);
    while args.len() < argc && i < buf.len() {
        let start = i;
        while i < buf.len() && buf[i] != 0 {
            i += 1;
        }
        args.push(String::from_utf8_lossy(&buf[start..i]).into_owned());
        i += 1;
    }
    args
}

/// `/etc/services` name for a port, via `getservbyport(3)`. That call uses a
/// static buffer, hence the lock.
pub fn service_for_port(port: u16, udp: bool) -> Option<String> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _g = LOCK.lock().ok()?;
    let proto: &CStr = if udp { c"udp" } else { c"tcp" };
    // SAFETY: getservbyport takes the port in network byte order; the result
    // points into libc's static storage, read before the lock is released.
    unsafe {
        let s = libc::getservbyport((port.to_be()) as c_int, proto.as_ptr());
        if s.is_null() || (*s).s_name.is_null() {
            return None;
        }
        Some(CStr::from_ptr((*s).s_name).to_string_lossy().into_owned())
    }
}

/// Send `sig` to one pid (`group` = the group it leads). Returns errno on
/// failure. The guard in `stop.rs` decides whether this may be called.
pub fn signal(pid: i32, sig: c_int, group: bool) -> Result<(), i32> {
    // SAFETY: plain syscalls; pid > 1 is enforced by the caller's guard.
    let r = unsafe {
        if group {
            libc::killpg(pid, sig)
        } else {
            libc::kill(pid, sig)
        }
    };
    if r == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().raw_os_error().unwrap_or(0))
    }
}

/// Reap `pid` if it is our own exited child, so it stops answering
/// `kill(pid, 0)` as a zombie. Never blocks.
pub fn reap(pid: i32) {
    let mut status = 0;
    // SAFETY: WNOHANG; only affects our own children.
    unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn procargs_stops_before_the_environment() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&2i32.to_ne_bytes());
        buf.extend_from_slice(b"/usr/bin/python3\0\0\0\0");
        buf.extend_from_slice(b"python3\0-mhttp.server\0SECRET_TOKEN=abc\0HOME=/x\0");
        let args = parse_procargs(&buf);
        assert_eq!(args, vec!["python3", "-mhttp.server"]);
        assert!(!args.iter().any(|a| a.contains("SECRET")));
    }

    #[test]
    fn reads_itself() {
        let mut me = read_proc(my_pid()).expect("own process");
        fill_usage(&mut me);
        assert_eq!(me.uid, my_uid());
        assert!(me.start_sec > 0);
        assert!(me.footprint.unwrap_or(0) > 0, "footprint");
        assert!(me.cpu_ns.is_some());
        assert!(!me.path.is_empty());
    }

    #[test]
    fn known_port_names() {
        assert_eq!(service_for_port(5432, false).as_deref(), Some("postgresql"));
    }
}
