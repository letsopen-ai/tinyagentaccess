mod doctor;
mod procchain;
const DOCTOR_HELP: &str = "tinyagentaccess doctor [--json]\n\nPrint read-only diagnostics: version, binary path and SHA-256, parent process chain,\nlikely host app, session hints, authentication file status (path, existence, access;\nnever the token), and (macOS) Screen Recording / Accessibility preflight.\nNever prompts, creates or rotates tokens, or changes anything.\n\nExit codes: 0 ok or not applicable, 1 macOS permission preflight failed, 2 usage or doctor error.";
const HELP: &str = "tinyAgentAccess\n\nUsage: tinyagentaccess serve [--config <path>] [--bind <addr>] [--token-file <path>] [--allow-non-loopback]\n       tinyagentaccess print-config\n       tinyagentaccess doctor [--json]\n       tinyagentaccess rotate-token [--config <path>] [--token-file <path>]\n       tinyagentaccess observe-url [--config <path>] [--bind <addr>] [--token-file <path>]\n       tinyagentaccess --version\n\n/mcp requires `Authorization: Bearer <token>`; the token is read from a 0600 file (see README).";
use std::path::{Path, PathBuf};
use taa_tools::{auth, config::Config};

/// Options shared by `serve`, `observe-url` and `rotate-token`.
#[derive(Debug, Default, PartialEq, Eq)]
struct Options {
    config: Option<PathBuf>,
    bind: Option<String>,
    token_file: Option<PathBuf>,
    allow_non_loopback: bool,
    help: bool,
}

fn parse_options(
    command: &str,
    args: impl IntoIterator<Item = String>,
    allowed: &[&str],
) -> Result<Options, String> {
    let mut args = args.into_iter();
    let mut o = Options::default();
    while let Some(a) = args.next() {
        if matches!(a.as_str(), "--help" | "-h") {
            o.help = true;
        } else if !allowed.contains(&a.as_str()) {
            return Err(format!("unknown option {a} for {command}"));
        } else if a == "--allow-non-loopback" {
            o.allow_non_loopback = true;
        } else {
            let value = args.next().ok_or(format!("{a} needs a value"))?;
            match a.as_str() {
                "--config" => o.config = Some(value.into()),
                "--bind" => o.bind = Some(value),
                _ => o.token_file = Some(value.into()),
            }
        }
    }
    Ok(o)
}

/// A named `--config` path must be readable: there is no silent fallback to defaults.
fn read_config(path: Option<&Path>) -> Result<Option<String>, String> {
    path.map(|p| {
        std::fs::read_to_string(p).map_err(|e| format!("cannot read config {}: {e}", p.display()))
    })
    .transpose()
}

fn effective_config(text: Option<&str>, o: &Options) -> Result<Config, String> {
    let mut config = match text {
        Some(text) => toml::from_str(text).map_err(|e| e.to_string())?,
        None => Config::default(),
    };
    if let Some(bind) = &o.bind {
        config.bind = bind.clone();
    }
    config.auth.allow_non_loopback |= o.allow_non_loopback;
    config.validate()?;
    Ok(config)
}

fn load_config(o: &Options) -> Result<Config, String> {
    effective_config(read_config(o.config.as_deref())?.as_deref(), o)
}

/// The observer accepts only loopback Host headers, even when the server is
/// bound to a concrete non-loopback address (docs/design.md, Authentication).
/// For such a bind the printed URL is therefore the loopback side of an SSH
/// forward, and a note explains the forward. Returns (url, note).
fn observer_access(addr: std::net::SocketAddr, secret: &str) -> (String, Option<String>) {
    if addr.ip().is_loopback() {
        return (auth::observer_url(&addr.to_string(), secret), None);
    }
    let port = addr.port();
    let url = auth::observer_url(&format!("127.0.0.1:{port}"), secret);
    let note = format!(
        "The observer rejects non-loopback Host headers, so http://{addr}/observe will not work. Open it through an SSH forward from the machine with the browser: ssh -L {port}:{}:{port} user@host, then use the URL above. The supported setup is a loopback listener plus this tunnel.",
        addr.ip()
    );
    (url, Some(note))
}

fn observer_enabled(config: &Config) -> bool {
    config.agent_access.user_control_mode != "disabled"
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "--help".into());
    if matches!(command.as_str(), "--help" | "-h") {
        println!("{HELP}");
        return Ok(());
    }
    if matches!(command.as_str(), "--version" | "-V") {
        println!("tinyagentaccess {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if command == "doctor" {
        match doctor::parse_args(args) {
            Err(e) => {
                eprintln!("error: {e}\n{DOCTOR_HELP}");
                std::process::exit(2);
            }
            Ok(o) if o.help => {
                println!("{DOCTOR_HELP}");
                std::process::exit(0);
            }
            Ok(o) => std::process::exit(doctor::run(&o)),
        }
    }
    if command == "print-config" {
        print!("{}", toml::to_string_pretty(&Config::default())?);
        return Ok(());
    }
    match command.as_str() {
        "serve" => serve(args).await,
        "rotate-token" => rotate_token(args),
        "observe-url" => observe_url(args),
        _ => Err(format!("unknown command {command}").into()),
    }
}

fn rotate_token(args: impl Iterator<Item = String>) -> Result<(), Box<dyn std::error::Error>> {
    let o = parse_options("rotate-token", args, &["--config", "--token-file"])?;
    if o.help {
        println!("tinyagentaccess rotate-token [--config <path>] [--token-file <path>]\n\nReplaces the agent token file with a new random token. Prints the path, never the token.\nRestart any running server and update clients. The observer secret is not rotated.");
        return Ok(());
    }
    let config = load_config(&o)?;
    let paths = auth::resolve_paths(o.token_file.as_deref(), &config)?;
    auth::rotate_agent_token(&paths)?;
    println!("rotated agent token: {}", paths.agent.display());
    eprintln!("Restart any running tinyagentaccess server and update client configuration.");
    Ok(())
}

fn observe_url(args: impl Iterator<Item = String>) -> Result<(), Box<dyn std::error::Error>> {
    let o = parse_options("observe-url", args, &["--config", "--bind", "--token-file"])?;
    if o.help {
        println!("tinyagentaccess observe-url [--config <path>] [--bind <addr>] [--token-file <path>]\n\nPrints the observer URL, including its secret fragment, creating the secret file if needed.\nRequires agent_access.user_control_mode other than \"disabled\".");
        return Ok(());
    }
    let config = load_config(&o)?;
    if !observer_enabled(&config) {
        return Err(
            "the observer is disabled (agent_access.user_control_mode = \"disabled\")".into(),
        );
    }
    let paths = auth::resolve_paths(o.token_file.as_deref(), &config)?;
    let secret = auth::load_observer_secret(&paths)?;
    let addr: std::net::SocketAddr = config.bind.parse()?;
    let (url, note) = observer_access(addr, &secret);
    if let Some(note) = note {
        eprintln!("{note}");
    }
    println!("{url}");
    Ok(())
}

async fn serve(args: impl Iterator<Item = String>) -> Result<(), Box<dyn std::error::Error>> {
    let o = parse_options(
        "serve",
        args,
        &["--config", "--bind", "--token-file", "--allow-non-loopback"],
    )?;
    if o.help {
        println!("tinyagentaccess serve [--config <path>] [--bind <addr>] [--token-file <path>] [--allow-non-loopback]");
        return Ok(());
    }
    let config = load_config(&o)?;
    let paths = auth::resolve_paths(o.token_file.as_deref(), &config)?;
    let auth = auth::load(&paths, observer_enabled(&config))?;
    let addr: std::net::SocketAddr = config.bind.parse()?;
    if !addr.ip().is_loopback() {
        eprintln!("WARNING: non-loopback bind: there is no TLS, so tokens and screen contents cross the network in clear text. Do not expose this server to a network. The supported way to reach a remote machine is an SSH tunnel to a loopback listener: ssh -L 8765:127.0.0.1:8765 user@host");
    }
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let local = listener.local_addr()?;
    eprintln!(
        "tinyAgentAccess listening on http://{local}/mcp (auth required, token file: {})",
        paths.agent.display()
    );
    if let Some(secret) = auth.observer_secret() {
        // Explicit exception to "never print secrets": the user needs this URL once.
        let (url, note) = observer_access(local, secret);
        eprintln!(
            "Observer (open this URL, not the bare /observe page; secret file: {}): {url}",
            paths.observer.display()
        );
        if let Some(note) = note {
            eprintln!("{note}");
        }
    }
    let backend = config.platform.linux_backend.clone();
    let platform = tokio::task::spawn_blocking(move || taa_platform::detect(&backend)).await?;
    let server = taa_mcp::Server::new(config, platform, auth);
    axum::serve(listener, taa_mcp::router(server.clone()))
        .with_graceful_shutdown({
            let server = server.clone();
            async move {
                let _ = tokio::signal::ctrl_c().await;
                server.shutdown().await;
            }
        })
        .await?;
    server.shutdown().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bind(b: &str) -> Options {
        Options {
            bind: Some(b.into()),
            ..Options::default()
        }
    }

    #[test]
    fn bind_override_validates_effective_config() {
        let wildcard = "bind = '0.0.0.0:8765'";
        let safe = "127.0.0.1:8765";
        assert_eq!(
            effective_config(Some(wildcard), &bind(safe)).unwrap().bind,
            safe
        );
        assert!(effective_config(Some(wildcard), &Options::default()).is_err());
        assert!(Config::parse(wildcard).is_err());
        assert!(effective_config(None, &bind("0.0.0.0:8765")).is_err());
        for invalid in [
            "typo = 1",
            "[sessions]\nmax_sessions = 0",
            "[agent_access]\ncomputer_vision = false",
            "[auth]\ntypo = 1",
            "[auth]\ntoken_file = 'relative'",
        ] {
            assert!(effective_config(Some(invalid), &bind(safe)).is_err());
        }
    }

    #[test]
    fn wildcard_stays_rejected_even_with_the_opt_in() {
        let o = Options {
            bind: Some("0.0.0.0:8765".into()),
            allow_non_loopback: true,
            ..Options::default()
        };
        assert!(effective_config(None, &o).unwrap_err().contains("wildcard"));
        let o = Options {
            allow_non_loopback: true,
            bind: Some("[::]:8765".into()),
            ..Options::default()
        };
        assert!(effective_config(None, &o).is_err());
    }

    #[test]
    fn non_loopback_needs_the_flag_or_the_config_opt_in() {
        let concrete = "192.0.2.10:8765";
        let e = effective_config(None, &bind(concrete)).unwrap_err();
        assert!(
            e.contains("--allow-non-loopback") && e.contains("SSH"),
            "{e}"
        );
        let flag = Options {
            allow_non_loopback: true,
            ..bind(concrete)
        };
        assert_eq!(effective_config(None, &flag).unwrap().bind, concrete);
        let config = format!("bind = '{concrete}'\n[auth]\nallow_non_loopback = true");
        assert!(effective_config(Some(&config), &Options::default()).is_ok());
        assert!(
            !effective_config(None, &Options::default())
                .unwrap()
                .auth
                .allow_non_loopback
        );
    }

    #[test]
    fn observer_url_is_loopback_even_for_a_non_loopback_bind() {
        let secret = "s".repeat(64);
        let (url, note) = observer_access("127.0.0.1:8765".parse().unwrap(), &secret);
        assert_eq!(url, format!("http://127.0.0.1:8765/observe#{secret}"));
        assert!(note.is_none());
        let (url, note) = observer_access("192.0.2.10:9000".parse().unwrap(), &secret);
        assert_eq!(url, format!("http://127.0.0.1:9000/observe#{secret}"));
        let note = note.unwrap();
        assert!(note.contains("ssh -L 9000:192.0.2.10:9000") && !note.contains(&secret));
    }

    #[test]
    fn named_config_that_is_missing_is_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("nope.toml");
        let e = read_config(Some(&missing)).unwrap_err();
        assert!(e.contains("cannot read config"), "{e}");
        let o = Options {
            config: Some(missing),
            ..Options::default()
        };
        assert!(load_config(&o).is_err());
        // A directory is unreadable as a file, too.
        assert!(read_config(Some(tmp.path())).is_err());
        assert_eq!(read_config(None).unwrap(), None);
        let present = tmp.path().join("c.toml");
        std::fs::write(&present, "bind = '127.0.0.1:9000'").unwrap();
        let o = Options {
            config: Some(present),
            ..Options::default()
        };
        assert_eq!(load_config(&o).unwrap().bind, "127.0.0.1:9000");
    }

    #[test]
    fn option_parsing_is_strict_per_command() {
        let args = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let o = parse_options(
            "serve",
            args(&["--token-file", "/t", "--allow-non-loopback", "--bind", "b"]),
            &["--bind", "--token-file", "--allow-non-loopback"],
        )
        .unwrap();
        assert_eq!(o.token_file, Some("/t".into()));
        assert!(o.allow_non_loopback);
        assert_eq!(o.bind.as_deref(), Some("b"));
        assert!(parse_options("rotate-token", args(&["--bind", "x"]), &["--token-file"]).is_err());
        assert!(parse_options("serve", args(&["--bind"]), &["--bind"]).is_err());
        assert!(parse_options("serve", args(&["-h"]), &[]).unwrap().help);
        assert!(parse_options("serve", args(&["--insecure-no-auth"]), &["--bind"]).is_err());
    }

    #[test]
    fn explicit_missing_token_file_is_not_created() {
        let tmp = tempfile::tempdir().unwrap();
        let typo = tmp.path().join("mcp-tokne");
        let paths = auth::resolve_paths(Some(&typo), &Config::default()).unwrap();
        assert!(auth::load(&paths, false).is_err());
        assert!(auth::rotate_agent_token(&paths).is_err());
        assert!(!typo.exists());
        assert!(!paths.observer.exists());
    }
}
