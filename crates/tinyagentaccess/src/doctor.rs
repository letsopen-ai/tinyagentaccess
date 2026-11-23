//! `tinyagentaccess doctor`: read-only, no-prompt diagnostics.
//!
//! Never requests permissions, captures, posts input, binds a port, writes
//! files, or reads the TCC database. macOS facts come only from
//! `taa_platform::preflight`.
use crate::procchain::{self, Proc};
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use taa_platform::preflight::{self, MacosPreflight};

#[derive(Debug, PartialEq, Eq)]
pub struct Options {
    pub json: bool,
    pub help: bool,
}

pub fn parse_args<I: IntoIterator<Item = String>>(args: I) -> Result<Options, String> {
    let mut o = Options {
        json: false,
        help: false,
    };
    for a in args {
        match a.as_str() {
            "--json" => o.json = true,
            "--help" | "-h" => o.help = true,
            _ => return Err(format!("unknown doctor option {a}")),
        }
    }
    Ok(o)
}

/// Everything the report needs; built from the real system or from fakes.
#[derive(Debug)]
pub struct Facts {
    pub version: String,
    pub binary_path: String,
    pub binary_sha256: String,
    pub os: &'static str,
    pub platform_backend: String,
    pub macos: Option<MacosPreflight>,
    pub chain: Vec<Proc>,
    pub ssh: bool,
    pub no_gui: bool,
}

fn hex(digest: &[u8]) -> String {
    let mut s = String::with_capacity(digest.len() * 2);
    for b in digest {
        let _ = write!(s, "{b:02x}");
    }
    s
}

#[cfg(test)]
fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn hash_file(path: &std::path::Path) -> Result<String, String> {
    use std::io::Read;
    let mut f =
        std::fs::File::open(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let mut h = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = f
            .read(&mut buf)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    let s = hex(&h.finalize());
    Ok(s)
}

fn backend_summary(macos: &Option<MacosPreflight>) -> String {
    if let Some(m) = macos {
        return m.backend.to_string();
    }
    if cfg!(target_os = "linux") {
        preflight::linux_session_from_env().to_string()
    } else if cfg!(windows) {
        "windows-gdi-sendinput".to_string()
    } else {
        "unavailable".to_string()
    }
}

pub fn gather() -> Result<Facts, String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot locate running binary: {e}"))?;
    let exe = std::fs::canonicalize(&exe).unwrap_or(exe);
    let binary_sha256 = hash_file(&exe)?;
    let env = |k: &str| std::env::var_os(k).is_some();
    let macos = preflight::macos();
    Ok(Facts {
        version: env!("CARGO_PKG_VERSION").into(),
        binary_path: exe.display().to_string(),
        binary_sha256,
        os: std::env::consts::OS,
        platform_backend: backend_summary(&macos),
        macos,
        chain: procchain::ancestors(std::process::id()),
        ssh: env("SSH_CONNECTION") || env("SSH_CLIENT") || env("SSH_TTY"),
        no_gui: cfg!(target_os = "linux") && !env("DISPLAY") && !env("WAYLAND_DISPLAY"),
    })
}

const TERMINALS: &[&str] = &[
    "terminal",
    "iterm",
    "iterm2",
    "ghostty",
    "kitty",
    "alacritty",
    "wezterm",
    "wezterm-gui",
    "warp",
    "hyper",
    "tabby",
    "konsole",
    "gnome-terminal",
    "gnome-terminal-",
    "xterm",
    "windowsterminal",
    "wt",
    "conhost",
    "openconsole",
    "mintty",
    "tmux: server",
];
const GUI_HOSTS: &[&str] = &[
    "code",
    "code helper",
    "cursor",
    "electron",
    "claude",
    "zed",
    "idea",
    "pycharm",
    "xcode",
    "emacs",
    "vim",
    "nvim",
    "explorer",
];
/// System supervisors are never a permission host.
const SUPERVISORS: &[&str] = &[
    "launchd",
    "init",
    "systemd",
    "tini",
    "dumb-init",
    "runit",
    "s6-svscan",
    "supervisord",
];
fn base(name: &str) -> String {
    let n = name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(name)
        .to_lowercase();
    n.strip_suffix(".exe").map(str::to_string).unwrap_or(n)
}

/// Heuristic, nearest ancestor wins within a class: skip tinyagentaccess itself;
/// prefer the nearest known terminal; else the nearest known GUI host; else
/// none. sshd, supervisors and unknown ancestors are never a host app (SSH is
/// only a session hint).
/// The chain is ordered from this process's parent outward.
pub fn likely_host_app(chain: &[Proc]) -> Option<String> {
    let rest: Vec<&Proc> = chain
        .iter()
        .filter(|p| {
            let b = base(&p.name);
            !b.starts_with("tinyagentaccess") && !SUPERVISORS.contains(&b.as_str())
        })
        .collect();
    let pick = |set: &[&str]| rest.iter().find(|p| set.contains(&base(&p.name).as_str()));
    pick(TERMINALS)
        .or_else(|| pick(GUI_HOSTS))
        .map(|p| p.name.clone())
}

pub fn hints(f: &Facts) -> Vec<String> {
    let mut h = Vec::new();
    if f.ssh {
        h.push("SSH environment variables are set; a headless SSH process may not have access to the logged-in user's desktop.".into());
    }
    if f.no_gui {
        h.push(
            "Neither DISPLAY nor WAYLAND_DISPLAY is set; this looks like a non-graphical session."
                .into(),
        );
    }
    if f.os == "macos" && !f.ssh && f.chain.iter().any(|p| base(&p.name).starts_with("sshd")) {
        h.push(
            "An sshd ancestor was found; capture and input need the logged-in desktop session."
                .into(),
        );
    }
    h
}

pub fn remedy(f: &Facts, host: Option<&str>) -> String {
    let app = host.unwrap_or("the terminal or app that launches tinyagentaccess");
    let mut s = format!("Permissions belong to {app}, not to a Rust source file. A new path or a new binary may need a new grant. Quit and restart {app} (and the server) after changing permissions. Capture and input need an unlocked interactive desktop.");
    if f.os == "macos" {
        let _ = write!(s, " On macOS, enable Screen Recording and Accessibility for {app} in System Settings > Privacy & Security.");
    }
    s
}

/// 0 = ok / not applicable, 1 = a macOS preflight check failed.
/// ScreenCaptureKit absence is informational only.
pub fn exit_code(macos: &Option<MacosPreflight>) -> i32 {
    match macos {
        Some(m) if !m.screen_recording || !m.accessibility => 1,
        _ => 0,
    }
}

fn esc(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(o, "\\u{:04x}", c as u32);
            }
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

pub fn render_json(f: &Facts) -> String {
    let host = likely_host_app(&f.chain);
    let macos = match &f.macos {
        Some(m) => format!(
            "{{\"screen_recording\":{},\"accessibility\":{},\"screencapturekit_available\":{}}}",
            m.screen_recording, m.accessibility, m.screencapturekit_available
        ),
        None => "null".into(),
    };
    let chain: Vec<String> = f
        .chain
        .iter()
        .map(|p| format!("{{\"pid\":{},\"name\":{}}}", p.pid, esc(&p.name)))
        .collect();
    let hints: Vec<String> = hints(f).iter().map(|h| esc(h)).collect();
    format!(
        "{{\"version\":{},\"binary_path\":{},\"binary_sha256\":{},\"os\":{},\"platform_backend\":{},\"macos\":{},\"process_chain\":[{}],\"likely_host_app\":{},\"hints\":[{}],\"remedy\":{}}}\n",
        esc(&f.version),
        esc(&f.binary_path),
        esc(&f.binary_sha256),
        esc(f.os),
        esc(&f.platform_backend),
        macos,
        chain.join(","),
        host.as_deref().map_or("null".into(), esc),
        hints.join(","),
        esc(&remedy(f, host.as_deref())),
    )
}

pub fn render_text(f: &Facts) -> String {
    let host = likely_host_app(&f.chain);
    let mut s = String::new();
    let _ = writeln!(s, "tinyagentaccess doctor");
    let _ = writeln!(s, "version: {}", f.version);
    let _ = writeln!(s, "binary: {}", f.binary_path);
    let _ = writeln!(s, "sha256: {}", f.binary_sha256);
    let _ = writeln!(s, "os: {}", f.os);
    let _ = writeln!(s, "platform backend: {}", f.platform_backend);
    if f.os == "windows" {
        let _ = writeln!(s, "  Windows GDI/SendInput backend is compiled in; capture and input need an unlocked interactive desktop.");
    }
    if let Some(m) = &f.macos {
        let yn = |b: bool| if b { "granted" } else { "NOT granted" };
        let _ = writeln!(
            s,
            "Screen Recording (preflight): {}",
            yn(m.screen_recording)
        );
        let _ = writeln!(s, "Accessibility (preflight): {}", yn(m.accessibility));
        let _ = writeln!(
            s,
            "ScreenCaptureKit available: {}",
            if m.screencapturekit_available {
                "yes"
            } else {
                "no"
            }
        );
    }
    let _ = writeln!(s, "process chain (parent first):");
    if f.chain.is_empty() {
        let _ = writeln!(s, "  (unavailable)");
    }
    for p in &f.chain {
        let _ = writeln!(s, "  {} {}", p.pid, p.name);
    }
    let _ = writeln!(
        s,
        "likely host app: {}",
        host.as_deref().unwrap_or("unknown")
    );
    for h in hints(f) {
        let _ = writeln!(s, "hint: {h}");
    }
    let _ = writeln!(s, "remedy: {}", remedy(f, host.as_deref()));
    s
}

/// Runs the report and returns the process exit code.
pub fn run(opts: &Options) -> i32 {
    match gather() {
        Ok(f) => {
            if opts.json {
                print!("{}", render_json(&f));
            } else {
                print!("{}", render_text(&f));
            }
            exit_code(&f.macos)
        }
        Err(e) => {
            eprintln!("error: {e}");
            2
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chain(names: &[&str]) -> Vec<Proc> {
        names
            .iter()
            .enumerate()
            .map(|(i, n)| Proc {
                pid: 100 - i as u32,
                name: (*n).into(),
            })
            .collect()
    }
    fn facts() -> Facts {
        Facts {
            version: "0.1.1".into(),
            binary_path: "/x/tinyagentaccess".into(),
            binary_sha256: "ab".repeat(32),
            os: "linux",
            platform_backend: "x11".into(),
            macos: None,
            chain: chain(&["zsh", "iTerm2", "launchd"]),
            ssh: false,
            no_gui: false,
        }
    }
    fn mac(s: bool, a: bool, k: bool) -> Option<MacosPreflight> {
        Some(MacosPreflight {
            screen_recording: s,
            accessibility: a,
            screencapturekit_available: k,
            backend: "macos-screencapturekit",
        })
    }

    #[test]
    fn host_app_heuristic() {
        assert_eq!(
            likely_host_app(&chain(&["zsh", "iTerm2", "launchd"])).as_deref(),
            Some("iTerm2")
        );
        assert_eq!(
            likely_host_app(&chain(&["tinyagentaccess", "bash", "Terminal"])).as_deref(),
            Some("Terminal")
        );
        assert_eq!(
            likely_host_app(&chain(&["bash", "sshd", "sshd", "init"])),
            None
        );
        assert_eq!(
            likely_host_app(&chain(&["sh", "Code Helper", "Code"])).as_deref(),
            Some("Code Helper")
        );
        assert_eq!(likely_host_app(&chain(&["bash", "mytool", "init"])), None);
        assert_eq!(likely_host_app(&chain(&["tinyagentaccess"])), None);
        assert_eq!(likely_host_app(&[]), None);
        assert_eq!(
            likely_host_app(&chain(&["cmd.exe", "WindowsTerminal.exe"])).as_deref(),
            Some("WindowsTerminal.exe")
        );
    }

    #[test]
    fn hints_for_ssh_and_headless() {
        let mut f = facts();
        assert!(hints(&f).is_empty());
        f.ssh = true;
        f.no_gui = true;
        let h = hints(&f);
        assert_eq!(h.len(), 2);
        assert!(h[0].contains("SSH"));
        assert!(h[1].contains("DISPLAY"));
    }

    #[test]
    fn exit_code_policy() {
        assert_eq!(exit_code(&None), 0);
        assert_eq!(exit_code(&mac(true, true, true)), 0);
        assert_eq!(exit_code(&mac(true, true, false)), 0);
        assert_eq!(exit_code(&mac(false, true, true)), 1);
        assert_eq!(exit_code(&mac(true, false, true)), 1);
        assert_eq!(exit_code(&mac(false, false, false)), 1);
    }

    #[test]
    fn arg_parsing_is_strict() {
        assert_eq!(
            parse_args(vec![]).unwrap(),
            Options {
                json: false,
                help: false
            }
        );
        assert!(parse_args(vec!["--json".into()]).unwrap().json);
        assert!(parse_args(vec!["-h".into()]).unwrap().help);
        assert!(parse_args(vec!["--bogus".into()]).is_err());
        assert!(parse_args(vec!["--json".into(), "extra".into()]).is_err());
    }

    #[test]
    fn remedy_names_host() {
        let f = facts();
        let r = remedy(&f, Some("iTerm2"));
        assert!(r.contains("Permissions belong to iTerm2"));
        assert!(r.contains("unlocked interactive desktop"));
        assert!(!r.contains("System Settings"));
        let mut m = facts();
        m.os = "macos";
        let r = remedy(&m, None);
        assert!(r.contains("Screen Recording and Accessibility"));
        assert!(r.contains("System Settings > Privacy & Security"));
    }

    #[test]
    fn json_shape_linux() {
        let mut f = facts();
        f.ssh = true;
        let j = render_json(&f);
        assert!(j.ends_with("}\n"));
        for key in [
            "\"version\":\"0.1.1\"",
            "\"binary_path\":\"/x/tinyagentaccess\"",
            "\"binary_sha256\":\"abab",
            "\"os\":\"linux\"",
            "\"platform_backend\":\"x11\"",
            "\"macos\":null",
            "\"process_chain\":[{\"pid\":100,\"name\":\"zsh\"},{\"pid\":99,\"name\":\"iTerm2\"},{\"pid\":98,\"name\":\"launchd\"}]",
            "\"likely_host_app\":\"iTerm2\"",
            "\"hints\":[\"SSH environment",
            "\"remedy\":\"Permissions belong to iTerm2",
        ] {
            assert!(j.contains(key), "missing {key} in {j}");
        }
    }

    #[test]
    fn json_shape_macos_and_escaping() {
        let mut f = facts();
        f.os = "macos";
        f.macos = mac(true, false, true);
        f.chain = chain(&["we\"ird\\name"]);
        let j = render_json(&f);
        assert!(j.contains("\"macos\":{\"screen_recording\":true,\"accessibility\":false,\"screencapturekit_available\":true}"));
        assert!(j.contains("we\\\"ird\\\\name"));
        let none = Facts {
            chain: vec![],
            ..facts()
        };
        assert!(render_json(&none).contains("\"likely_host_app\":null"));
    }

    #[test]
    fn text_report_contents() {
        let t = render_text(&facts());
        for s in [
            "version: 0.1.1",
            "binary: /x/tinyagentaccess",
            "sha256: abab",
            "platform backend: x11",
            "likely host app: iTerm2",
            "remedy:",
        ] {
            assert!(t.contains(s), "missing {s}");
        }
        assert!(!t.contains("Screen Recording (preflight)"));
    }

    #[test]
    fn sha256_known_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn supervisors_are_never_the_host() {
        let mut f = facts();
        f.os = "macos";
        let ch = chain(&["zsh", "sshd-session", "launchd"]);
        assert_eq!(likely_host_app(&ch), None);
        let only = chain(&["launchd"]);
        assert_eq!(likely_host_app(&only), None);
        assert_eq!(likely_host_app(&chain(&["zsh", "systemd"])), None);
        assert_eq!(likely_host_app(&chain(&["bash", "tini"])), None);
        f.chain = only;
        let r = remedy(&f, likely_host_app(&f.chain).as_deref());
        assert!(!r.contains("launchd"));
        assert!(r.contains("the terminal or app that launches tinyagentaccess"));
        f.chain = ch;
        let r = remedy(&f, likely_host_app(&f.chain).as_deref());
        assert!(!r.contains("sshd-session") && !r.contains("sshd") && !r.contains("launchd"));
        assert!(r.contains("the terminal or app that launches tinyagentaccess"));
        f.ssh = true;
        assert!(hints(&f).iter().any(|h| h.contains("SSH")));
        f.ssh = false;
        assert!(hints(&f).iter().any(|h| h.contains("sshd ancestor")));
    }

    /// Normalizes CRLF (Windows checkouts) to LF so marker searches are
    /// line-ending independent.
    fn lf(src: &str) -> String {
        src.replace("\r\n", "\n")
    }

    /// Finds `needle` in `hay`, naming the needle on failure.
    fn find_or_panic(hay: &str, needle: &str) -> usize {
        hay.find(needle)
            .unwrap_or_else(|| panic!("scan marker not found: {needle:?}"))
    }

    /// Production code only: everything before the `mod tests` module.
    /// Input must already be LF-normalized.
    fn prod(src: &str) -> &str {
        &src[..find_or_panic(src, "#[cfg(test)]\nmod tests")]
    }

    /// The `if command == "doctor" { ... }` dispatch block of `main.rs`.
    /// Input must already be LF-normalized.
    fn doctor_dispatch(main: &str) -> &str {
        let b = find_or_panic(main, "if command == \"doctor\" {");
        let e = b + find_or_panic(&main[b..], "\n    }\n");
        &main[b..e]
    }

    /// The scan markers must keep working if sources are checked out with
    /// CRLF, so this cannot regress only on Windows.
    #[test]
    fn doctor_scan_markers_survive_crlf_sources() {
        for raw in [
            include_str!("doctor.rs"),
            include_str!("procchain.rs"),
            include_str!("main.rs"),
        ] {
            let crlf = raw.replace("\r\n", "\n").replace('\n', "\r\n");
            // Raw CRLF text defeats the LF-only needles; normalization fixes it.
            assert!(!crlf.contains("#[cfg(test)]\nmod tests"));
            let norm = lf(&crlf);
            assert_eq!(norm, lf(raw));
            if norm.contains("#[cfg(test)]\nmod tests") {
                assert!(prod(&norm).len() < norm.len());
            }
        }
        let main_crlf = include_str!("main.rs")
            .replace("\r\n", "\n")
            .replace('\n', "\r\n");
        assert!(!main_crlf.contains("\n    }\n"));
        let norm = lf(&main_crlf);
        assert!(doctor_dispatch(&norm).contains("doctor::run"));
        let doctor_crlf = include_str!("doctor.rs")
            .replace("\r\n", "\n")
            .replace('\n', "\r\n");
        let doctor_norm = lf(&doctor_crlf);
        // The test module must be excluded from the production slice.
        assert!(!prod(&doctor_norm).contains("fn doctor_sources_have_no_prompt_apis"));
    }

    /// The doctor path must never reach request/prompt APIs, input, capture,
    /// servers, or the TCC database. The macOS wrappers, FFI and class probe
    /// are scanned in `taa-platform`'s `preflight` tests.
    #[test]
    fn doctor_sources_have_no_prompt_apis() {
        const BANNED: &[&str] = &[
            "CGRequestScreenCaptureAccess",
            "CGRequestPostEventAccess",
            "AXIsProcessTrustedWithOptions",
            "kAXTrustedCheckOptionPrompt",
            "TCC.db",
            "NSWorkspace",
            "x-apple.systempreferences",
            "CGEventPost",
            "SCShareableContent",
            "SCScreenshotManager",
            "TcpListener",
            "taa_platform::detect",
            "Command::new",
            "fs::write",
            "File::create",
        ];
        let main = lf(include_str!("main.rs"));
        let doctor_all = lf(include_str!("doctor.rs"));
        let procchain_all = lf(include_str!("procchain.rs"));
        let doctor_src = prod(&doctor_all);
        // Guard against the scan silently shrinking again.
        assert!(doctor_src.contains("pub fn run("));
        assert!(doctor_src.contains("fn hash_file"));
        assert!(doctor_src.contains("pub fn render_text"));
        for src in [doctor_src, prod(&procchain_all), doctor_dispatch(&main)] {
            for b in BANNED {
                assert!(!src.contains(b), "forbidden `{b}` in doctor path");
            }
        }
    }
}
