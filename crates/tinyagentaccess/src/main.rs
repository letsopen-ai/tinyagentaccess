use std::path::PathBuf;
use taa_tools::config::Config;
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "--help".into());
    if matches!(command.as_str(), "--help" | "-h") {
        println!("tinyAgentAccess\n\nUsage: tinyagentaccess serve [--config <path>] [--bind <addr>]\n       tinyagentaccess print-config\n       tinyagentaccess --version");
        return Ok(());
    }
    if matches!(command.as_str(), "--version" | "-V") {
        println!("tinyagentaccess {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if command == "print-config" {
        print!("{}", toml::to_string_pretty(&Config::default())?);
        return Ok(());
    }
    if command != "serve" {
        return Err(format!("unknown command {command}").into());
    }
    let (mut path, mut bind) = (None::<PathBuf>, None);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--config" => path = Some(args.next().ok_or("--config needs a path")?.into()),
            "--bind" => bind = Some(args.next().ok_or("--bind needs an address")?),
            "--help" | "-h" => {
                println!("tinyagentaccess serve [--config <path>] [--bind <addr>]");
                return Ok(());
            }
            _ => return Err(format!("unknown option {a}").into()),
        }
    }
    let config_text = match path {
        Some(p) => match std::fs::read_to_string(p) {
            Ok(s) => Some(s),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        },
        None => None,
    };
    let config = effective_config(config_text.as_deref(), bind)?;
    let addr: std::net::SocketAddr = config.bind.parse()?;
    if !addr.ip().is_loopback() {
        eprintln!("WARNING: NO AUTHENTICATION: non-loopback bind exposes desktop control. Do not expose this server.");
    }
    let listener = tokio::net::TcpListener::bind(addr).await?;
    eprintln!(
        "tinyAgentAccess listening on http://{}/mcp (NO AUTH)",
        listener.local_addr()?
    );
    let backend = config.platform.linux_backend.clone();
    let platform = tokio::task::spawn_blocking(move || taa_platform::detect(&backend)).await?;
    let server = taa_mcp::Server::new(config, platform);
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

fn effective_config(text: Option<&str>, bind: Option<String>) -> Result<Config, String> {
    let mut config = match text {
        Some(text) => toml::from_str(text).map_err(|e| e.to_string())?,
        None => Config::default(),
    };
    if let Some(bind) = bind {
        config.bind = bind;
    }
    config.validate()?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bind_override_validates_effective_config() {
        let wildcard = "bind = '0.0.0.0:8765'";
        let safe = "127.0.0.1:8765";
        assert_eq!(
            effective_config(Some(wildcard), Some(safe.into()))
                .unwrap()
                .bind,
            safe
        );
        assert!(effective_config(Some(wildcard), None).is_err());
        assert!(Config::parse(wildcard).is_err());
        assert!(effective_config(None, Some("0.0.0.0:8765".into())).is_err());
        for invalid in [
            "typo = 1",
            "[sessions]\nmax_sessions = 0",
            "[agent_access]\ncomputer_vision = false",
        ] {
            assert!(effective_config(Some(invalid), Some(safe.into())).is_err());
        }
    }
}
