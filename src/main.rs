use std::{env, path::PathBuf, sync::Arc, time::Instant};

use anyhow::{Context, Result, bail};
use mc_proxy::{
    AppConfig, CrossplayProvider, RuntimeManager, ViaLiteRuntime, api::ApiState,
    geyser_lite::CrossplayRuntime, validate_admin_token, web,
};
use tokio::net::TcpListener;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

const HELP: &str = "\
mc-proxy - Minecraft Java TCP proxy and admin panel

Usage:
  mc-proxy [--config <Path>]
  mc-proxy --version
  mc-proxy --help

Environment variables:
  MC_PROXY_ADMIN_TOKEN  Required, at least 32 characters, for admin API access
  MC_PROXY_UPDATE_STATUS_PATH  Optional path to the updater status JSON file
  RUST_LOG              Optional, for example mc_proxy=debug

Configuration:
  Defaults to config.toml when --config is omitted.
  If the file does not exist, the built-in default is written. See config.example.toml.
";

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    let config_path = parse_args()?;
    init_tracing();

    let admin_token = env::var("MC_PROXY_ADMIN_TOKEN")
        .context("Missing MC_PROXY_ADMIN_TOKEN environment variable")?;
    validate_admin_token(&admin_token)?;
    let update_status_path = env::var_os("MC_PROXY_UPDATE_STATUS_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/var/lib/mc-proxy/update-status.json"));

    let (config, loaded_path) = AppConfig::load(config_path.as_deref())?;
    let admin_listen = config.admin.listen;
    let via_runtime = ViaLiteRuntime::new();
    if let Err(error) = via_runtime.apply(&config).await {
        warn!(%error, "ViaLite failed to start; the proxy will connect directly to the backend. Check ViaLite status in the console");
    }
    let manager = Arc::new(
        RuntimeManager::new(config.clone(), loaded_path.clone())
            .with_via_dial_targets(via_runtime.dial_targets()),
    );
    manager.start().await?;

    let crossplay_runtime = CrossplayRuntime::new();
    crossplay_runtime.apply(&config.crossplay).await?;
    if config.crossplay.enabled && config.crossplay.provider == CrossplayProvider::GeyserLite {
        let runtime_status = crossplay_runtime.status().await;
        if runtime_status.running {
            info!("GeyserLite translation layer started");
        } else if let Some(error) = runtime_status.error.as_deref() {
            warn!(%error, "GeyserLite failed to start; the console will show the error");
        }
    }

    let listener = TcpListener::bind(admin_listen)
        .await
        .with_context(|| format!("Cannot bind admin listener at {admin_listen}"))?;
    info!(
        %admin_listen,
        config = %loaded_path.display(),
        "Minecraft proxy admin server started"
    );

    let state = ApiState {
        manager: Arc::clone(&manager),
        admin_token: Arc::from(admin_token),
        started_at: Instant::now(),
        crossplay_runtime,
        via_runtime,
        update_status_path,
    };

    let result = axum::serve(listener, web::router(state.clone()))
        .with_graceful_shutdown(shutdown_signal())
        .await;
    state.crossplay_runtime.stop().await;
    state.via_runtime.stop().await;
    manager.shutdown().await;
    result.context("Admin server exited unexpectedly")
}

fn parse_args() -> Result<Option<PathBuf>> {
    let mut args = env::args_os().skip(1);
    let mut config_path = None;

    while let Some(argument) = args.next() {
        match argument.to_str() {
            Some("-V" | "--version") => {
                println!("{}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            Some("-h" | "--help") => {
                print!("{HELP}");
                std::process::exit(0);
            }
            Some("-c" | "--config") => {
                let Some(path) = args.next() else {
                    bail!("--config requires a file path");
                };
                if config_path.replace(PathBuf::from(path)).is_some() {
                    bail!("--config may only be specified once");
                }
            }
            Some(other) => bail!("Unknown argument: {other}\n\n{HELP}"),
            None => bail!("Parameters are not valid UTF-8"),
        }
    }

    Ok(config_path)
}

fn init_tracing() {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("mc_proxy=info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .compact()
        .init();
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};

        let mut terminate =
            signal(SignalKind::terminate()).expect("Unable to register SIGTERM listener");
        tokio::select! {
            result = tokio::signal::ctrl_c() => {
                if let Err(error) = result {
                    tracing::error!(%error, "Failed to listen for Ctrl+C");
                }
            }
            _ = terminate.recv() => {}
        }
    }

    #[cfg(not(unix))]
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::error!(%error, "Failed to listen for Ctrl+C");
    }
}
