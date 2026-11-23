#[cfg(target_os = "linux")]
use sha2::{Digest, Sha256};
use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tinyagentaccess"))
        .args(args)
        .output()
        .unwrap()
}
fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn help_mentions_doctor_and_aliases_work() {
    for flag in ["--help", "-h"] {
        let o = run(&[flag]);
        assert!(o.status.success());
        assert!(out(&o).contains("tinyagentaccess doctor [--json]"));
    }
    for flag in ["--version", "-V"] {
        let o = run(&[flag]);
        assert!(o.status.success());
        assert_eq!(
            out(&o),
            format!("tinyagentaccess {}\n", env!("CARGO_PKG_VERSION"))
        );
    }
    let o = run(&["print-config"]);
    assert!(o.status.success());
    assert!(out(&o).contains("bind"));
}

#[test]
fn doctor_help_and_usage_errors() {
    for flag in ["--help", "-h"] {
        let o = run(&["doctor", flag]);
        assert_eq!(o.status.code(), Some(0));
        assert!(out(&o).contains("doctor"));
    }
    let o = run(&["doctor", "--bogus"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("--bogus"));
    assert!(out(&o).is_empty());
    assert_eq!(run(&["nonsense"]).status.code(), Some(1));
}

#[cfg(not(target_os = "macos"))]
#[test]
fn doctor_text_report() {
    let o = run(&["doctor"]);
    assert_eq!(o.status.code(), Some(0));
    let t = out(&o);
    assert!(t.contains(&format!("version: {}", env!("CARGO_PKG_VERSION"))));
    let exe = std::fs::canonicalize(env!("CARGO_BIN_EXE_tinyagentaccess")).unwrap();
    assert!(t.contains(&format!("binary: {}", exe.display())));
    assert!(t.contains("sha256: "));
    assert!(t.contains("platform backend: "));
    assert!(t.contains("remedy: "));
    assert!(!t.contains("Screen Recording (preflight)"));
}

#[cfg(target_os = "linux")]
#[test]
fn doctor_json_report_linux() {
    let o = Command::new(env!("CARGO_BIN_EXE_tinyagentaccess"))
        .args(["doctor", "--json"])
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("XDG_SESSION_TYPE")
        .env("SSH_CONNECTION", "1.2.3.4 1 5.6.7.8 22")
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(0));
    let t = out(&o);
    assert!(t.starts_with('{') && t.trim_end().ends_with('}'));
    assert_eq!(t.lines().count(), 1);
    for key in [
        "\"version\":\"",
        "\"binary_path\":\"/",
        "\"os\":\"linux\"",
        "\"platform_backend\":\"unavailable\"",
        "\"macos\":null",
        "\"process_chain\":[",
        "\"likely_host_app\":",
        "\"hints\":[\"SSH environment",
        "\"remedy\":\"",
    ] {
        assert!(t.contains(key), "missing {key} in {t}");
    }
    // Single JSON object, parsed for real.
    let v: serde_json::Value = serde_json::from_str(&t).expect("stdout is one JSON object");
    let obj = v.as_object().unwrap();
    for k in [
        "version",
        "binary_path",
        "binary_sha256",
        "os",
        "platform_backend",
        "remedy",
    ] {
        assert!(obj[k].is_string(), "{k} must be a string");
    }
    assert!(obj["macos"].is_null());
    assert!(obj["process_chain"].is_array());
    assert!(obj["hints"].is_array());
    assert!(obj["likely_host_app"].is_null() || obj["likely_host_app"].is_string());
    let sha = obj["binary_sha256"].as_str().unwrap();
    assert_eq!(sha.len(), 64);
    assert!(sha
        .bytes()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
    // Expected digest computed in-process; no external utility needed.
    let bytes =
        std::fs::read(std::fs::canonicalize(env!("CARGO_BIN_EXE_tinyagentaccess")).unwrap())
            .unwrap();
    let expected: String = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(sha, expected);
}
