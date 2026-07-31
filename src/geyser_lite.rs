#[cfg(all(feature = "geyserlite", target_os = "linux"))]
use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

#[cfg(all(feature = "geyserlite", target_os = "linux"))]
use anyhow::{Context, Result, anyhow};
use serde::Serialize;
#[cfg(all(feature = "geyserlite", target_os = "linux"))]
use tokio::{sync::Mutex, task::JoinHandle, time::timeout};

use crate::GeyserLiteMode;
#[cfg(all(feature = "geyserlite", target_os = "linux"))]
use crate::{CrossplayAuthType, CrossplayConfig, CrossplayProvider};

/// GeyserLite 托管运行状态（与构建特性无关，供 API 与控制台展示）。
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct GeyserLiteRuntimeStatus {
    /// 当前构建是否编译了 geyserlite 特性。
    pub available: bool,
    /// 配置是否要求托管 GeyserLite（enabled 且 provider = geyserlite）。
    pub enabled: bool,
    pub running: bool,
    pub mode: Option<GeyserLiteMode>,
    pub error: Option<String>,
}

#[cfg(all(feature = "geyserlite", target_os = "linux"))]
mod imp {
    use super::*;
    use tracing::{error, info, warn};

    #[derive(Clone)]
    pub struct CrossplayRuntime {
        inner: Arc<Inner>,
    }

    struct Inner {
        managed: Mutex<Option<Managed>>,
        status: Arc<Mutex<GeyserLiteRuntimeStatus>>,
        generation: Arc<AtomicU64>,
    }

    struct Managed {
        server: Arc<geyserlite::Server>,
        fingerprint: CrossplayConfig,
        task: JoinHandle<()>,
    }

    impl CrossplayRuntime {
        pub fn new() -> Self {
            Self {
                inner: Arc::new(Inner {
                    managed: Mutex::new(None),
                    status: Arc::new(Mutex::new(GeyserLiteRuntimeStatus {
                        available: true,
                        enabled: false,
                        running: false,
                        mode: None,
                        error: None,
                    })),
                    generation: Arc::new(AtomicU64::new(0)),
                }),
            }
        }

        /// 根据配置启动、停止或重启托管 GeyserLite。
        ///
        /// 启动失败不会返回 Err，而是记录到 [`Self::status`]，避免配置已持久化后
        /// 管理 API 出现“配置与运行态不一致”的假象。
        pub async fn apply(&self, config: &CrossplayConfig) -> Result<()> {
            let want_managed = config.enabled && config.provider == CrossplayProvider::GeyserLite;
            let mut managed = self.inner.managed.lock().await;
            let mut status = self.inner.status.lock().await;

            if !want_managed {
                if let Some(existing) = managed.take() {
                    drop(status);
                    existing.stop().await;
                    status = self.inner.status.lock().await;
                }
                status.enabled = false;
                status.running = false;
                status.mode = None;
                status.error = None;
                return Ok(());
            }

            status.enabled = true;
            if let Some(existing) = managed.as_ref()
                && existing.fingerprint == *config
            {
                return Ok(());
            }
            if let Some(_existing) = managed.as_ref()
                && config.geyserlite.mode == GeyserLiteMode::Embedded
            {
                // embedded 与 mc-proxy 共享地址空间，进程内二次启动 Geyser 原生
                // 桥接不可靠（实测会把整个进程带崩），因此已运行实例不做热更新。
                status.mode = Some(GeyserLiteMode::Embedded);
                status.error = Some(
                    "embedded 模式不支持热更新：已保留当前实例运行，请重启 mc-proxy \
                     使新配置生效；需要在线热更新请改用 subprocess 模式"
                        .to_string(),
                );
                warn!("embedded 模式配置已保存但未热应用，需要重启 mc-proxy 生效");
                return Ok(());
            }

            let existing = managed.take();
            if let Some(existing) = existing {
                drop(status);
                existing.stop().await;
                status = self.inner.status.lock().await;
            }

            let options = build_options(config)?;
            let server = tokio::task::spawn_blocking({
                let options = options;
                move || geyserlite::Server::new(options)
            })
            .await
            .map_err(|error| anyhow!("GeyserLite 初始化任务异常: {error}"))?
            .map_err(|error| anyhow!("初始化 GeyserLite 失败: {error}"))?;
            let server = Arc::new(server);

            let generation = self.inner.generation.fetch_add(1, Ordering::SeqCst) + 1;
            let task_server = Arc::clone(&server);
            let task_status = Arc::clone(&self.inner.status);
            let task_generation = Arc::clone(&self.inner.generation);
            let task = tokio::spawn(async move {
                run_server(task_server, task_status, task_generation, generation).await;
            });

            status.running = true;
            status.mode = Some(config.geyserlite.mode);
            status.error = None;
            *managed = Some(Managed {
                server,
                fingerprint: config.clone(),
                task,
            });
            info!(
                mode = ?config.geyserlite.mode,
                listen = %config.bedrock_listen,
                upstream = %format!("{}:{}", config.java_address, config.java_port),
                "GeyserLite 托管翻译层已启动"
            );
            Ok(())
        }

        pub async fn status(&self) -> GeyserLiteRuntimeStatus {
            self.inner.status.lock().await.clone()
        }

        pub async fn stop(&self) {
            let mut managed = self.inner.managed.lock().await;
            if let Some(existing) = managed.take() {
                existing.stop().await;
            }
            let mut status = self.inner.status.lock().await;
            status.enabled = false;
            status.running = false;
            status.mode = None;
            status.error = None;
        }
    }

    impl Default for CrossplayRuntime {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Managed {
        async fn stop(mut self) {
            self.server.stop().await;
            match timeout(Duration::from_secs(10), &mut self.task).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => error!(%error, "GeyserLite 任务异常退出"),
                Err(_) => {
                    warn!("GeyserLite 停止超时，强制取消托管任务");
                    self.task.abort();
                    let _ = self.task.await;
                }
            }
        }
    }

    async fn run_server(
        server: Arc<geyserlite::Server>,
        status: Arc<Mutex<GeyserLiteRuntimeStatus>>,
        generation: Arc<AtomicU64>,
        mine: u64,
    ) {
        match server.start().await {
            Ok(()) => {
                // 正常结束说明收到了 stop 请求；只有当前代次才能改写状态。
                let mut status = status.lock().await;
                if generation.load(Ordering::SeqCst) == mine {
                    status.running = false;
                    status.error = None;
                }
            }
            Err(error) => {
                let mut status = status.lock().await;
                if generation.load(Ordering::SeqCst) == mine {
                    status.running = false;
                    status.error = Some(format!("GeyserLite 退出: {error}"));
                }
                error!(%error, "GeyserLite 托管实例退出");
            }
        }
    }

    fn build_options(config: &CrossplayConfig) -> Result<geyserlite::Options> {
        let geyserlite_config = &config.geyserlite;
        let mode = match geyserlite_config.mode {
            GeyserLiteMode::Embedded => geyserlite::Mode::Embedded,
            GeyserLiteMode::Subprocess => geyserlite::Mode::Subprocess,
        };
        let floodgate_key = match config.auth_type {
            CrossplayAuthType::Floodgate => {
                let hex = geyserlite_config
                    .floodgate_key
                    .as_deref()
                    .context("Floodgate 认证需要 geyserlite.floodgate_key")?;
                decode_hex_key(hex)?
            }
            _ => Vec::new(),
        };
        Ok(geyserlite::Options {
            listen: config.bedrock_listen.to_string(),
            upstream: format!("{}:{}", config.java_address, config.java_port),
            auth_type: match config.auth_type {
                CrossplayAuthType::Online => geyserlite::AuthType::Online,
                CrossplayAuthType::Floodgate => geyserlite::AuthType::Floodgate,
                CrossplayAuthType::Offline => geyserlite::AuthType::Offline,
            },
            floodgate_key,
            motd: geyserlite::Motd {
                line1: geyserlite_config.motd_line1.clone(),
                line2: geyserlite_config.motd_line2.clone(),
            },
            mode,
            library_path: geyserlite_config.library_path.clone(),
            binary_path: geyserlite_config.binary_path.clone(),
            offline: geyserlite_config.offline,
            ..Default::default()
        })
    }

    fn decode_hex_key(hex: &str) -> Result<Vec<u8>> {
        let hex = hex.trim();
        if hex.len() != 32 || !hex.chars().all(|character| character.is_ascii_hexdigit()) {
            anyhow::bail!("floodgate_key 必须是 16 字节密钥的 32 位十六进制字符串");
        }
        (0..hex.len())
            .step_by(2)
            .map(|index| {
                u8::from_str_radix(&hex[index..index + 2], 16)
                    .map_err(|error| anyhow!("解析 floodgate_key 失败: {error}"))
            })
            .collect()
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn decodes_floodgate_hex_key() {
            let key = decode_hex_key("00112233445566778899aabbccddeeff").unwrap();
            assert_eq!(key.len(), 16);
            assert_eq!(key[0], 0x00);
            assert_eq!(key[15], 0xff);
        }

        #[test]
        fn rejects_invalid_floodgate_key() {
            assert!(decode_hex_key("00112233445566778899aabbccddee").is_err());
            assert!(decode_hex_key("zz112233445566778899aabbccddeeff").is_err());
        }

        #[test]
        fn build_options_maps_typed_fields() {
            let config = CrossplayConfig {
                enabled: true,
                provider: CrossplayProvider::GeyserLite,
                bedrock_listen: "0.0.0.0:19132".parse().unwrap(),
                java_address: "bedrock.example.com".to_string(),
                java_port: 25565,
                auth_type: CrossplayAuthType::Offline,
                geyserlite: crate::GeyserLiteConfig {
                    mode: GeyserLiteMode::Subprocess,
                    binary_path: Some("/opt/geyserlite/bin/geyserlite".to_string()),
                    offline: true,
                    motd_line1: "Line One".to_string(),
                    motd_line2: "Line Two".to_string(),
                    ..Default::default()
                },
            };
            let options = build_options(&config).unwrap();
            assert_eq!(options.listen, "0.0.0.0:19132");
            assert_eq!(options.upstream, "bedrock.example.com:25565");
            assert!(options.offline);
            assert_eq!(
                options.binary_path.as_deref(),
                Some("/opt/geyserlite/bin/geyserlite")
            );
        }
    }
}

#[cfg(not(all(feature = "geyserlite", target_os = "linux")))]
mod imp {
    use super::*;
    use crate::CrossplayConfig;
    use anyhow::Result;

    #[derive(Clone)]
    pub struct CrossplayRuntime;

    impl CrossplayRuntime {
        pub fn new() -> Self {
            Self
        }

        pub async fn apply(&self, _config: &CrossplayConfig) -> Result<()> {
            Ok(())
        }

        pub async fn status(&self) -> GeyserLiteRuntimeStatus {
            GeyserLiteRuntimeStatus {
                available: false,
                enabled: false,
                running: false,
                mode: None,
                error: Some("当前平台/构建未启用 GeyserLite 特性".to_string()),
            }
        }

        pub async fn stop(&self) {}
    }

    impl Default for CrossplayRuntime {
        fn default() -> Self {
            Self::new()
        }
    }
}

pub use imp::CrossplayRuntime;
