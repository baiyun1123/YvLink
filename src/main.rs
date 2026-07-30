use std::{env, path::PathBuf, sync::Arc, time::Instant};

use anyhow::{Context, Result, bail};
use mc_proxy::{AppConfig, RuntimeManager, api::ApiState, validate_admin_token, web};
use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::EnvFilter;

const HELP: &str = "\
mc-proxy - Minecraft Java TCP 转发器与管理面板

用法:
  mc-proxy [--config <路径>]
  mc-proxy --help

环境变量:
  MC_PROXY_ADMIN_TOKEN  必填，至少 32 个字符，用于管理 API 登录
  RUST_LOG              可选，例如 mc_proxy=debug

配置:
  未指定 --config 时使用当前目录的 config.toml；
  文件不存在则写入内置默认配置。可参考 config.example.toml。
";

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    let config_path = parse_args()?;
    init_tracing();

    let admin_token =
        env::var("MC_PROXY_ADMIN_TOKEN").context("缺少 MC_PROXY_ADMIN_TOKEN 环境变量")?;
    validate_admin_token(&admin_token)?;

    let (config, loaded_path) = AppConfig::load(config_path.as_deref())?;
    let admin_listen = config.admin.listen;
    let manager = Arc::new(RuntimeManager::new(config, loaded_path.clone()));
    manager.start().await?;

    let listener = TcpListener::bind(admin_listen)
        .await
        .with_context(|| format!("管理端无法监听 {admin_listen}"))?;
    info!(
        %admin_listen,
        config = %loaded_path.display(),
        "Minecraft 转发管理端已启动"
    );

    let state = ApiState {
        manager: Arc::clone(&manager),
        admin_token: Arc::from(admin_token),
        started_at: Instant::now(),
    };

    let result = axum::serve(listener, web::router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await;
    manager.shutdown().await;
    result.context("管理端服务异常退出")
}

fn parse_args() -> Result<Option<PathBuf>> {
    let mut args = env::args_os().skip(1);
    let mut config_path = None;

    while let Some(argument) = args.next() {
        match argument.to_str() {
            Some("-h" | "--help") => {
                print!("{HELP}");
                std::process::exit(0);
            }
            Some("-c" | "--config") => {
                let Some(path) = args.next() else {
                    bail!("--config 后必须提供文件路径");
                };
                if config_path.replace(PathBuf::from(path)).is_some() {
                    bail!("--config 只能指定一次");
                }
            }
            Some(other) => bail!("未知参数: {other}\n\n{HELP}"),
            None => bail!("参数不是有效 UTF-8"),
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

        let mut terminate = signal(SignalKind::terminate()).expect("无法注册 SIGTERM 监听器");
        tokio::select! {
            result = tokio::signal::ctrl_c() => {
                if let Err(error) = result {
                    tracing::error!(%error, "监听 Ctrl+C 失败");
                }
            }
            _ = terminate.recv() => {}
        }
    }

    #[cfg(not(unix))]
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::error!(%error, "监听 Ctrl+C 失败");
    }
}
