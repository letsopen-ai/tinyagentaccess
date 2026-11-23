//! Best-effort ancestor walk for `doctor`. Fails soft: a truncated chain is
//! returned as-is. Bounded by depth and cycle checks so it cannot hang.
//! Reads only process metadata; never reads TCC or any permission database.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proc {
    pub pid: u32,
    pub name: String,
}

const MAX_DEPTH: usize = 32;

/// Ancestors of `pid`, parent first (excluding `pid` itself).
pub fn ancestors(pid: u32) -> Vec<Proc> {
    walk(pid, platform::lookup)
}

/// `lookup(pid) -> (parent pid, name)`; `None` ends the chain.
fn walk(start: u32, lookup: impl Fn(u32) -> Option<(u32, String)>) -> Vec<Proc> {
    let mut out: Vec<Proc> = Vec::new();
    let mut seen = vec![start];
    let mut cur = start;
    while out.len() < MAX_DEPTH {
        let Some((ppid, _)) = lookup(cur) else { break };
        if ppid == 0 || seen.contains(&ppid) {
            break;
        }
        let Some((_, name)) = lookup(ppid) else {
            out.push(Proc {
                pid: ppid,
                name: "?".into(),
            });
            break;
        };
        seen.push(ppid);
        out.push(Proc { pid: ppid, name });
        cur = ppid;
    }
    out
}

#[cfg(target_os = "linux")]
mod platform {
    pub fn lookup(pid: u32) -> Option<(u32, String)> {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        parse_stat(&stat)
    }
    /// `pid (comm) S ppid ...`; comm may contain spaces and parentheses.
    pub(super) fn parse_stat(stat: &str) -> Option<(u32, String)> {
        let open = stat.find('(')?;
        let close = stat.rfind(')')?;
        let comm = stat.get(open + 1..close)?.to_string();
        let ppid = stat
            .get(close + 1..)?
            .split_whitespace()
            .nth(1)?
            .parse()
            .ok()?;
        Some((ppid, comm))
    }
}

#[cfg(target_os = "macos")]
mod platform {
    pub fn lookup(pid: u32) -> Option<(u32, String)> {
        // SAFETY: proc_pidinfo writes at most `size` bytes into a zeroed
        // proc_bsdinfo that we own; it is a read-only process query.
        unsafe {
            let mut info: libc::proc_bsdinfo = std::mem::zeroed();
            let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
            let n = libc::proc_pidinfo(
                pid as libc::c_int,
                libc::PROC_PIDTBSDINFO,
                0,
                &mut info as *mut _ as *mut libc::c_void,
                size,
            );
            if n != size {
                return None;
            }
            let raw = if info.pbi_name[0] != 0 {
                &info.pbi_name[..]
            } else {
                &info.pbi_comm[..]
            };
            let bytes: Vec<u8> = raw
                .iter()
                .take_while(|c| **c != 0)
                .map(|c| *c as u8)
                .collect();
            Some((info.pbi_ppid, String::from_utf8_lossy(&bytes).into_owned()))
        }
    }
}

#[cfg(windows)]
mod platform {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };

    pub fn lookup(pid: u32) -> Option<(u32, String)> {
        // SAFETY: standard Toolhelp snapshot enumeration; entry is zeroed with
        // dwSize set as the API requires, and the handle is closed before return.
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snap == INVALID_HANDLE_VALUE {
                return None;
            }
            let mut entry: PROCESSENTRY32W = std::mem::zeroed();
            entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
            let mut found = None;
            let mut ok = Process32FirstW(snap, &mut entry);
            while ok != 0 {
                if entry.th32ProcessID == pid {
                    let len = entry
                        .szExeFile
                        .iter()
                        .position(|c| *c == 0)
                        .unwrap_or(entry.szExeFile.len());
                    found = Some((
                        entry.th32ParentProcessID,
                        String::from_utf16_lossy(&entry.szExeFile[..len]),
                    ));
                    break;
                }
                ok = Process32NextW(snap, &mut entry);
            }
            CloseHandle(snap);
            found
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
mod platform {
    pub fn lookup(_: u32) -> Option<(u32, String)> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walks_and_truncates_softly() {
        let table = |pid: u32| match pid {
            30 => Some((20, "tinyagentaccess".to_string())),
            20 => Some((10, "zsh".to_string())),
            10 => Some((1, "Terminal".to_string())),
            _ => None,
        };
        let c = walk(30, table);
        let pids: Vec<u32> = c.iter().map(|p| p.pid).collect();
        assert_eq!(pids, vec![20, 10, 1]);
        assert_eq!(c[2].name, "?");
    }

    #[test]
    fn cycles_and_depth_terminate() {
        let cyc = |pid: u32| Some((if pid == 2 { 3 } else { 2 }, format!("p{pid}")));
        assert!(walk(3, cyc).len() <= MAX_DEPTH);
        let deep = |pid: u32| Some((pid + 1, format!("p{pid}")));
        assert_eq!(walk(1, deep).len(), MAX_DEPTH);
        assert!(walk(5, |_| None).is_empty());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn parses_proc_stat() {
        let (ppid, name) = platform::parse_stat("42 (my (weird) cmd) S 7 42 42 0").unwrap();
        assert_eq!((ppid, name.as_str()), (7, "my (weird) cmd"));
        assert!(platform::parse_stat("garbage").is_none());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn real_chain_has_parent() {
        assert!(!ancestors(std::process::id()).is_empty());
    }
}
