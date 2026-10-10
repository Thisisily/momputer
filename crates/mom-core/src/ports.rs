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
    #[cfg(windows)]
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

#[cfg(all(test, windows))]
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
}
