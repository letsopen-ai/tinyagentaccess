use std::{path::PathBuf, sync::Arc};
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
    let mut config = match path {
        Some(p) => match std::fs::read_to_string(p) {
            Ok(s) => Config::parse(&s)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Config::default(),
            Err(e) => return Err(e.into()),
        },
        None => Config::default(),
    };
    if let Some(b) = bind {
        config.bind = b;
    }
    config.validate()?;
    let addr: std::net::SocketAddr = config.bind.parse()?;
    if !addr.ip().is_loopback() {
        eprintln!("WARNING: NO AUTHENTICATION: non-loopback bind exposes desktop control. Do not expose this server.");
    }
    let listener = tokio::net::TcpListener::bind(addr).await?;
    eprintln!(
        "tinyAgentAccess listening on http://{}/mcp (NO AUTH; desktop backend unavailable)",
        listener.local_addr()?
    );
    axum::serve(
        listener,
        taa_mcp::router(taa_mcp::Server::new(
            config,
            Arc::new(taa_platform::UnavailablePlatform::default()),
        )),
    )
    .with_graceful_shutdown(async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await?;
    Ok(())
}
