//! ViaLite 的受管 subprocess 适配层。
//!
//! ViaLite 必须位于 YvLink 已完成域名选路之后、真实 Java 后端之前。每个真实后端
//! 获得一个仅回环监听的 ViaLite 入口；代理仍以真实后端为健康检查与展示对象，只有
//! 实际拨号会改到对应的本地入口。

use std::{
    collections::HashMap,
    net::{SocketAddr, TcpListener as StdTcpListener},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow, bail};
use serde::Serialize;
use tokio::{
    net::TcpStream,
    process::{Child, Command},
    sync::{Mutex, RwLock},
    time::{Instant, sleep},
};
use tracing::{info, warn};

use crate::AppConfig;

const READY_TIMEOUT: Duration = Duration::from_secs(15);
const READY_RETRY: Duration = Duration::from_millis(100);

/// API 与控制台使用的 ViaLite 运行状态。路径与实际后端地址不暴露给未认证访问者。
#[derive(Clone, Debug, Serialize)]
pub struct ViaLiteRuntimeStatus {
    pub available: bool,
    pub enabled: bool,
    pub running: bool,
    pub managed_backends: usize,
    pub error: Option<String>,
}

#[derive(Default)]
struct ViaLiteInner {
    child: Option<Child>,
    config_path: Option<PathBuf>,
    enabled: bool,
    managed_backends: usize,
    error: Option<String>,
}

/// 仅使用 ViaLite 的 subprocess 运行模式，避免原生运行时崩溃影响代理进程。
#[derive(Clone)]
pub struct ViaLiteRuntime {
    inner: Arc<Mutex<ViaLiteInner>>,
    dial_targets: Arc<RwLock<HashMap<String, String>>>,
}

#[derive(Serialize)]
struct NativeConfig {
    bind: String,
    gate_protocol: String,
    backends: Vec<NativeBackend>,
}

#[derive(Serialize)]
struct NativeBackend {
    name: String,
    address: String,
    bind: String,
    version: String,
    detect: bool,
    forwarding: &'static str,
}

impl ViaLiteRuntime {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(ViaLiteInner::default())),
            dial_targets: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn dial_targets(&self) -> Arc<RwLock<HashMap<String, String>>> {
        Arc::clone(&self.dial_targets)
    }

    /// 以完整应用配置重建 ViaLite。失败时不会让代理崩溃，状态会明确呈现故障；由于
    /// 拨号映射被清空，代理会保守地直连后端，避免将连接发送给已退出的回环端口。
    pub async fn apply(&self, config: &AppConfig) -> Result<()> {
        config.via.validate()?;
        self.stop().await;

        let mut inner = self.inner.lock().await;
        inner.enabled = config.via.enabled;
        inner.error = None;
        inner.managed_backends = 0;
        drop(inner);

        if !config.via.enabled {
            return Ok(());
        }

        let result = self.start(config).await;
        if let Err(error) = &result {
            let mut inner = self.inner.lock().await;
            inner.error = Some(error.to_string());
            warn!(%error, "ViaLite 托管运行时启动失败，代理将直连后端");
        }
        result
    }

    pub async fn status(&self) -> ViaLiteRuntimeStatus {
        let mut inner = self.inner.lock().await;
        let mut exited = false;
        let running = match inner.child.as_mut() {
            Some(child) => match child.try_wait() {
                Ok(None) => true,
                Ok(Some(status)) => {
                    inner.error = Some(format!("ViaLite 子进程已退出（{status}）"));
                    exited = true;
                    false
                }
                Err(error) => {
                    inner.error = Some(format!("无法读取 ViaLite 子进程状态: {error}"));
                    exited = true;
                    false
                }
            },
            None => false,
        };
        let status = ViaLiteRuntimeStatus {
            available: cfg!(target_os = "linux"),
            enabled: inner.enabled,
            running,
            managed_backends: inner.managed_backends,
            error: inner.error.clone(),
        };
        drop(inner);
        if exited {
            self.dial_targets.write().await.clear();
        }
        status
    }

    pub async fn stop(&self) {
        self.dial_targets.write().await.clear();
        let mut inner = self.inner.lock().await;
        if let Some(mut child) = inner.child.take() {
            if let Err(error) = child.start_kill() {
                warn!(%error, "停止 ViaLite 子进程失败");
            }
            if let Err(error) = child.wait().await {
                warn!(%error, "等待 ViaLite 子进程退出失败");
            }
        }
        if let Some(path) = inner.config_path.take()
            && let Err(error) = tokio::fs::remove_file(&path).await
            && error.kind() != std::io::ErrorKind::NotFound
        {
            warn!(path = %path.display(), %error, "清理 ViaLite 临时配置失败");
        }
        inner.managed_backends = 0;
    }

    async fn start(&self, config: &AppConfig) -> Result<()> {
        if !cfg!(target_os = "linux") {
            bail!("当前平台不支持 ViaLite 托管 subprocess；请在 Linux 上运行");
        }
        let binary = config
            .via
            .binary_path
            .as_deref()
            .ok_or_else(|| anyhow!("缺少 via.binary_path"))?;
        if !Path::new(binary).is_file() {
            bail!("ViaLite 可执行文件不存在: {binary}");
        }

        let (native_config, mappings) = build_native_config(config)?;
        if native_config.backends.is_empty() {
            bail!("启用 via 时至少需要一条已启用路由及其后端");
        }
        let runtime_dir = Path::new(&config.via.runtime_dir);
        tokio::fs::create_dir_all(runtime_dir)
            .await
            .with_context(|| format!("无法创建 ViaLite 运行目录 {}", runtime_dir.display()))?;
        let config_path = runtime_dir.join(format!("vialite-{}.json", unique_suffix()));
        let payload = serde_json::to_vec(&native_config).context("无法序列化 ViaLite 原生配置")?;
        tokio::fs::write(&config_path, payload)
            .await
            .with_context(|| format!("无法写入 ViaLite 配置 {}", config_path.display()))?;

        let mut command = Command::new(binary);
        command
            .arg("--config")
            .arg(&config_path)
            .kill_on_drop(true)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit());
        let mut child = command
            .spawn()
            .with_context(|| format!("无法启动 ViaLite 可执行文件 {binary}"))?;

        if let Err(error) = wait_until_ready(&mut child, mappings.values()).await {
            let _ = child.start_kill();
            let _ = child.wait().await;
            let _ = tokio::fs::remove_file(&config_path).await;
            return Err(error);
        }

        *self.dial_targets.write().await = mappings;
        let mut inner = self.inner.lock().await;
        inner.child = Some(child);
        inner.config_path = Some(config_path);
        inner.managed_backends = native_config.backends.len();
        info!(
            backends = inner.managed_backends,
            "ViaLite 托管协议兼容层已就绪"
        );
        Ok(())
    }
}

fn build_native_config(config: &AppConfig) -> Result<(NativeConfig, HashMap<String, String>)> {
    let mut backends = Vec::new();
    let mut mappings = HashMap::new();
    for rule in config.rules.iter().filter(|rule| rule.enabled) {
        for (index, address) in rule.backend.iter().enumerate() {
            if mappings.contains_key(address) {
                // 相同实际后端共用一个 ViaLite 入口，避免重复监听与不一致状态。
                continue;
            }
            let bind = free_loopback_address()?;
            let name = format!("{}-{}", rule.id, index);
            mappings.insert(address.clone(), bind.clone());
            backends.push(NativeBackend {
                name,
                address: address.clone(),
                bind,
                version: config.via.backend_version.clone(),
                detect: config.via.backend_version.eq_ignore_ascii_case("auto"),
                // YvLink 不实现 Velocity/Bungee 身份转发，因此明确使用 none。
                forwarding: "none",
            });
        }
    }
    Ok((
        NativeConfig {
            bind: "127.0.0.1:0".to_string(),
            gate_protocol: config.via.gate_protocol.clone(),
            backends,
        },
        mappings,
    ))
}

fn free_loopback_address() -> Result<String> {
    let listener = StdTcpListener::bind("127.0.0.1:0").context("无法分配 ViaLite 回环端口")?;
    let address: SocketAddr = listener.local_addr().context("无法读取 ViaLite 回环端口")?;
    drop(listener);
    Ok(address.to_string())
}

async fn wait_until_ready<'a>(
    child: &mut Child,
    addresses: impl Iterator<Item = &'a String>,
) -> Result<()> {
    let addresses: Vec<_> = addresses.cloned().collect();
    let deadline = Instant::now() + READY_TIMEOUT;
    loop {
        if let Some(status) = child.try_wait().context("无法读取 ViaLite 子进程状态")? {
            bail!("ViaLite 在就绪前退出（{status}）");
        }
        let mut all_ready = true;
        for address in &addresses {
            match tokio::time::timeout(Duration::from_millis(80), TcpStream::connect(address)).await
            {
                Ok(Ok(_)) => {}
                _ => {
                    all_ready = false;
                    break;
                }
            }
        }
        if all_ready {
            return Ok(());
        }
        if Instant::now() >= deadline {
            bail!(
                "ViaLite 在 {} 秒内未监听全部后端回环端口",
                READY_TIMEOUT.as_secs()
            );
        }
        sleep(READY_RETRY).await;
    }
}

fn unique_suffix() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

impl Default for ViaLiteRuntime {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_one_loopback_backend_per_unique_target() {
        let mut config = AppConfig::default();
        config.rules[0].backend = vec![
            "127.0.0.1:25566".to_string(),
            "127.0.0.1:25566".to_string(),
            "127.0.0.1:25567".to_string(),
        ];
        let (native, mappings) = build_native_config(&config).unwrap();
        assert_eq!(native.backends.len(), 2);
        assert_eq!(mappings.len(), 2);
        assert!(native.backends.iter().all(|backend| {
            backend.bind.starts_with("127.0.0.1:") && backend.forwarding == "none" && backend.detect
        }));
    }
}
