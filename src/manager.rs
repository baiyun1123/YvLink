use std::{collections::HashMap, path::PathBuf, sync::Arc};

use anyhow::{Context, Result, anyhow, bail};
use serde::Serialize;
use tokio::{
    sync::{Mutex, oneshot},
    task::JoinHandle,
};
use tracing::{error, info, warn};

use crate::{
    AppConfig, CrossplayConfig, ForwardConfig, GlobalSettings, Metrics, MetricsSnapshot,
    RuleConfig, ViaLiteConfig, create_listener, serve,
};

pub struct RuntimeManager {
    inner: Mutex<ManagerInner>,
    mutation: Mutex<()>,
    config_path: PathBuf,
    via_dial_targets: Arc<tokio::sync::RwLock<HashMap<String, String>>>,
}

struct ManagerInner {
    config: AppConfig,
    handle: Option<ProxyHandle>,
    metrics: Arc<Metrics>,
}

struct ProxyHandle {
    shutdown: Option<oneshot::Sender<()>>,
    task: JoinHandle<Result<()>>,
    forward: Arc<ForwardConfig>,
}

#[derive(Clone, Debug, Serialize)]
pub struct RuntimeStatus {
    pub totals: MetricsSnapshot,
    pub proxy_running: bool,
    pub rules: Vec<RuleStatus>,
}

#[derive(Clone, Debug, Serialize)]
pub struct RuleStatus {
    #[serde(flatten)]
    pub rule: RuleConfig,
    pub running: bool,
    pub backend_health: Vec<crate::BackendHealthSnapshot>,
}

impl RuntimeManager {
    pub fn new(config: AppConfig, config_path: PathBuf) -> Self {
        Self {
            inner: Mutex::new(ManagerInner {
                config,
                handle: None,
                metrics: Arc::new(Metrics::default()),
            }),
            mutation: Mutex::new(()),
            config_path,
            via_dial_targets: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        }
    }

    pub fn with_via_dial_targets(
        mut self,
        via_dial_targets: Arc<tokio::sync::RwLock<HashMap<String, String>>>,
    ) -> Self {
        self.via_dial_targets = via_dial_targets;
        self
    }

    pub async fn start(&self) -> Result<()> {
        let _mutation = self.mutation.lock().await;
        let mut inner = self.inner.lock().await;
        let config = inner.config.clone();
        if config.settings.proxy_enabled {
            let metrics = Arc::clone(&inner.metrics);
            inner.handle = Some(start_proxy(
                &config,
                metrics,
                Arc::clone(&self.via_dial_targets),
            )?);
        }
        if let Err(error) = config.persist(&self.config_path) {
            if let Some(handle) = inner.handle.take() {
                stop_proxy(handle).await;
            }
            return Err(error);
        }
        Ok(())
    }

    pub async fn config(&self) -> AppConfig {
        self.inner.lock().await.config.clone()
    }

    pub async fn status(&self) -> RuntimeStatus {
        let inner = self.inner.lock().await;
        let proxy_running = inner.handle.is_some();
        let forward = inner
            .handle
            .as_ref()
            .map(|handle| Arc::clone(&handle.forward));
        RuntimeStatus {
            totals: inner.metrics.snapshot(),
            proxy_running,
            rules: inner
                .config
                .rules
                .iter()
                .cloned()
                .map(|rule| RuleStatus {
                    running: proxy_running && rule.enabled,
                    backend_health: forward
                        .as_ref()
                        .map_or_else(Vec::new, |config| config.backend_health(&rule.id)),
                    rule,
                })
                .collect(),
        }
    }

    pub async fn update_settings(&self, settings: GlobalSettings) -> Result<AppConfig> {
        let _mutation = self.mutation.lock().await;
        let mut config = self.inner.lock().await.config.clone();
        config.settings = settings;
        self.apply_locked(config.clone()).await?;
        Ok(config)
    }

    pub async fn update_crossplay(&self, crossplay: CrossplayConfig) -> Result<AppConfig> {
        let _mutation = self.mutation.lock().await;
        crossplay.validate()?;
        let mut inner = self.inner.lock().await;
        let mut config = inner.config.clone();
        config.crossplay = crossplay;
        config.persist(&self.config_path)?;
        inner.config = config.clone();
        Ok(config)
    }

    pub async fn update_via(&self, via: ViaLiteConfig) -> Result<AppConfig> {
        let _mutation = self.mutation.lock().await;
        let mut inner = self.inner.lock().await;
        let mut config = inner.config.clone();
        config.via = via;
        config.validate()?;
        config.persist(&self.config_path)?;
        inner.config = config.clone();
        Ok(config)
    }

    pub async fn create_rule(&self, rule: RuleConfig) -> Result<RuleConfig> {
        let _mutation = self.mutation.lock().await;
        let mut config = self.inner.lock().await.config.clone();
        if config.rules.iter().any(|existing| existing.id == rule.id) {
            bail!("规则 id 已存在: {}", rule.id);
        }
        let insert_at = config
            .rules
            .iter()
            .position(is_enabled_catch_all)
            .unwrap_or(config.rules.len());
        config.rules.insert(insert_at, rule.clone());
        self.apply_locked(config).await?;
        Ok(rule)
    }

    pub async fn update_rule(&self, id: &str, mut rule: RuleConfig) -> Result<RuleConfig> {
        let _mutation = self.mutation.lock().await;
        let mut config = self.inner.lock().await.config.clone();
        let Some(index) = config.rules.iter().position(|existing| existing.id == id) else {
            bail!("规则不存在: {id}");
        };
        rule.id = id.to_string();
        config.rules[index] = rule.clone();
        self.apply_locked(config).await?;
        Ok(rule)
    }

    pub async fn delete_rule(&self, id: &str) -> Result<()> {
        let _mutation = self.mutation.lock().await;
        let mut config = self.inner.lock().await.config.clone();
        let previous = config.rules.len();
        config.rules.retain(|rule| rule.id != id);
        if config.rules.len() == previous {
            bail!("规则不存在: {id}");
        }
        self.apply_locked(config).await
    }

    pub async fn shutdown(&self) {
        let _mutation = self.mutation.lock().await;
        let handle = self.inner.lock().await.handle.take();
        if let Some(handle) = handle {
            stop_proxy(handle).await;
        }
    }

    async fn apply_locked(&self, new_config: AppConfig) -> Result<()> {
        new_config.validate()?;
        let mut inner = self.inner.lock().await;
        let old_config = inner.config.clone();
        if old_config == new_config {
            return Ok(());
        }

        let old_handle = inner.handle.take();
        if let Some(handle) = old_handle {
            stop_proxy(handle).await;
        }

        let new_handle = if new_config.settings.proxy_enabled {
            match start_proxy(
                &new_config,
                Arc::clone(&inner.metrics),
                Arc::clone(&self.via_dial_targets),
            ) {
                Ok(handle) => Some(handle),
                Err(error) => {
                    let metrics = Arc::clone(&inner.metrics);
                    inner.handle =
                        restart_old(&old_config, metrics, Arc::clone(&self.via_dial_targets))?;
                    return Err(error);
                }
            }
        } else {
            None
        };

        if let Err(error) = new_config.persist(&self.config_path) {
            if let Some(handle) = new_handle {
                stop_proxy(handle).await;
            }
            let metrics = Arc::clone(&inner.metrics);
            inner.handle = restart_old(&old_config, metrics, Arc::clone(&self.via_dial_targets))
                .context("新配置持久化失败，且旧入口回滚失败")?;
            return Err(error);
        }

        inner.config = new_config;
        inner.handle = new_handle;
        Ok(())
    }
}

fn is_enabled_catch_all(rule: &RuleConfig) -> bool {
    rule.enabled && rule.host.iter().any(|host| host.trim() == "*")
}

fn start_proxy(
    config: &AppConfig,
    metrics: Arc<Metrics>,
    via_dial_targets: Arc<tokio::sync::RwLock<HashMap<String, String>>>,
) -> Result<ProxyHandle> {
    let forward = Arc::new(ForwardConfig::from_app(config).with_via_dial_targets(via_dial_targets));
    let listener = create_listener(&forward)
        .with_context(|| format!("Minecraft 入口无法监听 {}", forward.listen))?;
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let listen = forward.listen;
    let route_count = forward.routes.len();
    let task_forward = Arc::clone(&forward);
    let task = tokio::spawn(async move {
        info!(%listen, route_count, "Minecraft 单端口域名路由入口已启动");
        let result = serve(listener, task_forward, metrics, async move {
            let _ = shutdown_rx.await;
        })
        .await;
        if let Err(error) = &result {
            error!(%error, "Minecraft 路由入口异常退出");
        }
        result
    });
    Ok(ProxyHandle {
        shutdown: Some(shutdown_tx),
        task,
        forward,
    })
}

fn restart_old(
    config: &AppConfig,
    metrics: Arc<Metrics>,
    via_dial_targets: Arc<tokio::sync::RwLock<HashMap<String, String>>>,
) -> Result<Option<ProxyHandle>> {
    if config.settings.proxy_enabled {
        start_proxy(config, metrics, via_dial_targets).map(Some)
    } else {
        Ok(None)
    }
}

async fn stop_proxy(mut handle: ProxyHandle) {
    if let Some(shutdown) = handle.shutdown.take() {
        let _ = shutdown.send(());
    }
    match handle.task.await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => warn!(%error, "Minecraft 路由入口退出时返回错误"),
        Err(error) => warn!(%error, "Minecraft 路由入口任务连接失败"),
    }
}

pub fn validate_admin_token(token: &str) -> Result<()> {
    if token.len() < 32 {
        bail!("MC_PROXY_ADMIN_TOKEN 至少需要 32 个字符");
    }
    if token.chars().any(char::is_whitespace) {
        return Err(anyhow!("MC_PROXY_ADMIN_TOKEN 不能包含空白字符"));
    }
    Ok(())
}
