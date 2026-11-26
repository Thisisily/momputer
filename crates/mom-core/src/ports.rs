//! Which process is listening on which TCP port.

use serde::Serialize;
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Listener {
    pub port: u16,
    pub addr: String,
    /// Bound to every interface, so other machines on the network can reach it.
    pub public: bool,
}

impl Listener {
    pub fn url(&self) -> String {
        format!("http://localhost:{}", self.port)
    }
}

/// pid -> ports it listens on (deduplicated across IPv4/IPv6, sorted).
pub fn listeners() -> HashMap<u32, Vec<Listener>> {
    let mut map: HashMap<u32, Vec<Listener>> = HashMap::new();
    #[cfg(any(windows, target_os = "macos"))]
    {
        for (pid, l) in imp::tcp_listeners() {
            let list = map.entry(pid).or_default();
            if let Some(existing) = list.iter_mut().find(|e| e.port == l.port) {
                existing.public |= l.public;
                if l.public {
                    existing.addr = l.addr;
                }
            } else {
                list.push(l);
            }
        }
    }
    for list in map.values_mut() {
        list.sort_by_key(|l| l.port);
    }
    map
}

#[cfg(windows)]
mod imp {
    use super::Listener;
    use std::net::{Ipv4Addr, Ipv6Addr};
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GetExtendedTcpTable, MIB_TCP6ROW_OWNER_PID, MIB_TCPROW_OWNER_PID, TCP_TABLE_OWNER_PID_LISTENER,
    };
    use windows_sys::Win32::Networking::WinSock::{AF_INET, AF_INET6};

    /// Calls GetExtendedTcpTable, growing a u32-aligned buffer until it fits.
    fn table(family: u16) -> Option<Vec<u32>> {
        let mut size: u32 = 0;
        unsafe {
            GetExtendedTcpTable(
                std::ptr::null_mut(),
                &mut size,
                0,
                family as u32,
                TCP_TABLE_OWNER_PID_LISTENER,
                0,
            );
        }
        for _ in 0..4 {
            let mut buf = vec![0u32; (size as usize).div_ceil(4) + 64];
            let mut len = (buf.len() * 4) as u32;
            let rc = unsafe {
                GetExtendedTcpTable(
                    buf.as_mut_ptr().cast(),
                    &mut len,
                    0,
                    family as u32,
                    TCP_TABLE_OWNER_PID_LISTENER,
                    0,
                )
            };
            match rc {
                0 => return Some(buf),
                122 => size = len, // ERROR_INSUFFICIENT_BUFFER: the table grew, try again
                _ => return None,
            }
        }
        None
    }

    pub fn tcp_listeners() -> Vec<(u32, Listener)> {
        let mut out = Vec::new();
        if let Some(buf) = table(AF_INET) {
            let n = buf[0] as usize;
            let rows = unsafe {
                std::slice::from_raw_parts(buf.as_ptr().add(1) as *const MIB_TCPROW_OWNER_PID, n)
            };
            for r in rows {
                let addr = Ipv4Addr::from(r.dwLocalAddr.to_ne_bytes());
                out.push((
                    r.dwOwningPid,
                    Listener {
                        port: u16::from_be((r.dwLocalPort & 0xFFFF) as u16),
                        public: addr.is_unspecified(),
                        addr: addr.to_string(),
                    },
                ));
            }
        }
        if let Some(buf) = table(AF_INET6) {
            let n = buf[0] as usize;
            let rows = unsafe {
                std::slice::from_raw_parts(buf.as_ptr().add(1) as *const MIB_TCP6ROW_OWNER_PID, n)
            };
            for r in rows {
                let addr = Ipv6Addr::from(r.ucLocalAddr);
                out.push((
                    r.dwOwningPid,
                    Listener {
                        port: u16::from_be((r.dwLocalPort & 0xFFFF) as u16),
                        public: addr.is_unspecified(),
                        addr: format!("[{addr}]"),
                    },
                ));
            }
        }
        out
    }
}

/// A Mac has no one table of listening ports to read. Each of your processes is asked for the
/// files it has open, and each socket among them for what it is: no program is run, and what
/// belongs to someone else isn't shown to us at all.
#[cfg(target_os = "macos")]
mod imp {
    use super::Listener;
    use std::ffi::{c_int, c_void};
    use std::net::{Ipv4Addr, Ipv6Addr};

    // <sys/proc_info.h>: what to ask about one open file, and where the answers are in
    // `struct socket_fdinfo` (792 bytes). The layout is part of macOS's public interface; the
    // test below opens ports of its own and finds them through it.
    const PROC_PIDFDSOCKETINFO: c_int = 3;
    const SOCKET_FDINFO_SIZE: usize = 792;
    const SOI_KIND: usize = 256;
    const INSI_LPORT: usize = 268;
    const INSI_VFLAG: usize = 288;
    const INSI_LADDR: usize = 312;
    const TCPSI_STATE: usize = 344;
    const SOCKINFO_TCP: i32 = 2;
    const TSI_S_LISTEN: i32 = 1;
    const INI_IPV4: u8 = 1;

    fn pids() -> Vec<c_int> {
        // SAFETY: first how many there are, then a buffer of ours that is as long as we say.
        unsafe {
            let n = libc::proc_listallpids(std::ptr::null_mut(), 0);
            if n <= 0 {
                return Vec::new();
            }
            let mut pids: Vec<c_int> = vec![0; n as usize + 64];
            let got = libc::proc_listallpids(pids.as_mut_ptr().cast::<c_void>(), (pids.len() * std::mem::size_of::<c_int>()) as c_int);
            pids.truncate(got.max(0) as usize);
            pids
        }
    }

    /// The numbers of the sockets a process has open. None of them for a process that isn't yours.
    fn sockets(pid: c_int, fds: &mut Vec<libc::proc_fdinfo>) -> impl Iterator<Item = i32> + '_ {
        let each = std::mem::size_of::<libc::proc_fdinfo>();
        // SAFETY: first how much room the list needs, then a buffer of ours with that much and
        // some to spare (more may have been opened since); what comes back is how much was filled.
        let filled = unsafe {
            let need = libc::proc_pidinfo(pid, libc::PROC_PIDLISTFDS, 0, std::ptr::null_mut(), 0);
            if need <= 0 {
                0
            } else {
                fds.clear();
                fds.resize(need as usize / each + 16, libc::proc_fdinfo { proc_fd: 0, proc_fdtype: 0 });
                libc::proc_pidinfo(pid, libc::PROC_PIDLISTFDS, 0, fds.as_mut_ptr().cast::<c_void>(), (fds.len() * each) as c_int).max(0) as usize / each
            }
        };
        fds.truncate(filled);
        fds.iter().filter(|f| f.proc_fdtype == libc::PROX_FDTYPE_SOCKET as u32).map(|f| f.proc_fd)
    }

    pub fn tcp_listeners() -> Vec<(u32, Listener)> {
        let mut out = Vec::new();
        let mut fds = Vec::new();
        // 792 bytes, lined up for the 64-bit numbers inside.
        let mut info = [0u64; SOCKET_FDINFO_SIZE / 8];
        for pid in pids() {
            if pid <= 0 {
                continue;
            }
            for fd in sockets(pid, &mut fds) {
                // SAFETY: the buffer is ours and exactly as long as we say; the kernel fills it
                // whole or says it didn't.
                let got = unsafe { libc::proc_pidfdinfo(pid, fd, PROC_PIDFDSOCKETINFO, info.as_mut_ptr().cast::<c_void>(), SOCKET_FDINFO_SIZE as c_int) };
                if got as usize != SOCKET_FDINFO_SIZE {
                    continue;
                }
                // SAFETY: the same 792 bytes, read as bytes.
                let bytes = unsafe { std::slice::from_raw_parts(info.as_ptr().cast::<u8>(), SOCKET_FDINFO_SIZE) };
                let int = |at: usize| i32::from_ne_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
                if int(SOI_KIND) != SOCKINFO_TCP || int(TCPSI_STATE) != TSI_S_LISTEN {
                    continue;
                }
                // The port is kept the way it travels: high byte first, in the low half.
                let port = u16::from_be(int(INSI_LPORT) as u16);
                if port == 0 {
                    continue;
                }
                let listener = if bytes[INSI_VFLAG] == INI_IPV4 {
                    let a = &bytes[INSI_LADDR + 12..INSI_LADDR + 16];
                    let addr = Ipv4Addr::new(a[0], a[1], a[2], a[3]);
                    Listener { port, public: addr.is_unspecified(), addr: addr.to_string() }
                } else {
                    // IPv6, or both at once (which is how a server that takes all comers listens).
                    let mut a = [0u8; 16];
                    a.copy_from_slice(&bytes[INSI_LADDR..INSI_LADDR + 16]);
                    let addr = Ipv6Addr::from(a);
                    Listener { port, public: addr.is_unspecified(), addr: format!("[{addr}]") }
                };
                out.push((pid as u32, listener));
            }
        }
        out
    }
}

#[cfg(all(test, any(windows, target_os = "macos")))]
mod tests {
    #[test]
    fn finds_a_port_we_open_ourselves() {
        let sock = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = sock.local_addr().unwrap().port();
        let map = super::listeners();
        let mine = map.get(&std::process::id()).expect("our own listener should show up");
        let l = mine.iter().find(|l| l.port == port).unwrap();
        assert!(!l.public);
        assert_eq!(l.url(), format!("http://localhost:{port}"));
    }

    #[test]
    fn tells_a_port_open_to_the_network_from_one_that_is_not() {
        let everyone = std::net::TcpListener::bind("0.0.0.0:0").unwrap();
        let six = std::net::TcpListener::bind("[::1]:0").ok();
        // Connected sockets and ones that aren't listening don't count.
        let client = std::net::TcpStream::connect(everyone.local_addr().unwrap()).ok();
        let map = super::listeners();
        let mine = map.get(&std::process::id()).expect("our own listeners should show up");
        let open = mine.iter().find(|l| l.port == everyone.local_addr().unwrap().port()).unwrap();
        assert!(open.public, "{open:?}");
        assert_eq!(open.addr, "0.0.0.0");
        if let Some(six) = six {
            let l = mine.iter().find(|l| l.port == six.local_addr().unwrap().port()).unwrap();
            assert!(!l.public);
            assert_eq!(l.addr, "[::1]");
        }
        if let Some(client) = client {
            assert!(!mine.iter().any(|l| l.port == client.local_addr().unwrap().port()));
        }
    }
}
