use std::{
    collections::{HashMap, HashSet},
    fs,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Deserializer, Serialize};
use tokio::sync::Mutex;

const MIN_COPY_BUFFER: usize = 4 * 1024;
const MAX_COPY_BUFFER: usize = 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct AppConfig {
    pub admin: AdminConfig,
    pub crossplay: CrossplayConfig,
    pub via: ViaLiteConfig,
    pub settings: GlobalSettings,
    pub rules: Vec<RuleConfig>,
}

/// ViaLite 在代理与 Java 后端之间提供的托管协议兼容层。
///
/// 仅支持由本程序管理的 subprocess 模式；它通过回环地址隔离原生运行时。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct ViaLiteConfig {
    pub enabled: bool,
    /// ViaLite 原生可执行文件。启用时必须是绝对路径，避免从不可信 PATH 查找。
    pub binary_path: Option<String>,
    /// 运行时 JSON 配置目录；systemd 部署默认使用 /run/mc-proxy/vialite。
    pub runtime_dir: String,
    /// YvLink 连接 ViaLite 时使用的 Java 协议，通常保持 auto。
    pub gate_protocol: String,
    /// 后端 Java 协议。auto 会交由 ViaLite/ViaProxy 识别。
    pub backend_version: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct AdminConfig {
    pub listen: SocketAddr,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct GlobalSettings {
    /// Minecraft Java 客户端共用的唯一入口。
    pub listen: SocketAddr,
    pub proxy_enabled: bool,
    pub max_connections: usize,
    pub connect_timeout_ms: u64,
    pub handshake_timeout_ms: u64,
    pub shutdown_grace_secs: u64,
    pub copy_buffer_bytes: usize,
    pub socket_buffer_bytes: usize,
    pub listen_backlog: i32,
    pub tcp_nodelay: bool,
    pub reuse_port: bool,
    pub stats_interval_secs: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct CrossplayConfig {
    pub enabled: bool,
    /// 翻译层提供方：external 使用独立 Geyser Standalone，geyserlite 由本代理托管。
    pub provider: CrossplayProvider,
    pub bedrock_listen: SocketAddr,
    pub java_address: String,
    pub java_port: u16,
    pub auth_type: CrossplayAuthType,
    /// provider = "geyserlite" 时的托管参数；external 模式下忽略。
    pub geyserlite: GeyserLiteConfig,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CrossplayProvider {
    #[default]
    External,
    #[serde(rename = "geyserlite")]
    GeyserLite,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum GeyserLiteMode {
    /// 与 mc-proxy 同一进程内加载 libgeyserlite.so（默认，开销最低）。
    #[default]
    Embedded,
    /// 以托管子进程方式运行 geyserlite，崩溃隔离更好。
    Subprocess,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct GeyserLiteConfig {
    pub mode: GeyserLiteMode,
    /// embedded 模式：libgeyserlite.so 的显式路径；留空走环境变量/系统路径/自动下载。
    pub library_path: Option<String>,
    /// subprocess 模式：geyserlite 原生可执行文件的显式路径；留空自动定位/下载。
    pub binary_path: Option<String>,
    /// 禁止任何网络获取；此时必须通过路径、环境变量或内嵌特性提供原生库。
    pub offline: bool,
    pub motd_line1: String,
    pub motd_line2: String,
    /// 仅 auth_type = "floodgate" 需要：16 字节密钥的 32 位十六进制字符串，敏感。
    pub floodgate_key: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CrossplayAuthType {
    #[default]
    Online,
    Floodgate,
    Offline,
}

/// 一条 Gate Lite 风格的 host -> backend 路由。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RuleConfig {
    pub id: String,
    pub name: String,
    #[serde(deserialize_with = "one_or_many")]
    pub host: Vec<String>,
    #[serde(deserialize_with = "one_or_many")]
    pub backend: Vec<String>,
    #[serde(default)]
    pub strategy: LoadBalancingStrategy,
    #[serde(default)]
    pub proxy_protocol: ProxyProtocolVersion,
    #[serde(default)]
    pub health_check: HealthCheckConfig,
    #[serde(default)]
    pub modify_virtual_host: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<StatusConfig>,
    #[serde(default)]
    pub whitelist_enabled: bool,
    #[serde(default)]
    pub whitelist: Vec<String>,
    #[serde(default = "default_whitelist_message")]
    pub whitelist_message: String,
    /// 是否允许全局 Bedrock Crossplay 入口把此规则作为 Java 上游。
    /// Crossplay 仍只有一个 UDP 监听入口，通过 java_address 选择其中一条允许的规则。
    #[serde(default)]
    pub crossplay_enabled: bool,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StatusConfig {
    #[serde(default)]
    pub mode: StatusMode,
    #[serde(default = "default_status_cache_ttl_secs")]
    pub cache_ttl_secs: i64,
    #[serde(default)]
    pub motd: Option<String>,
    #[serde(default)]
    pub version_name: Option<String>,
    #[serde(default)]
    pub protocol: Option<i32>,
    #[serde(default)]
    pub online: Option<u32>,
    #[serde(default)]
    pub max: Option<u32>,
    #[serde(default)]
    pub fallback: Option<StatusResponseConfig>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum StatusMode {
    #[default]
    Custom,
    Backend,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum LoadBalancingStrategy {
    #[default]
    Sequential,
    Random,
    RoundRobin,
    LeastConnections,
    LowestLatency,
}

#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ProxyProtocolVersion {
    #[default]
    Off,
    V1,
    V2,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct HealthCheckConfig {
    pub enabled: bool,
    pub mode: HealthCheckMode,
    pub interval_secs: u64,
    pub timeout_ms: u64,
    pub unhealthy_threshold: u32,
    pub healthy_threshold: u32,
    pub minecraft_host: Option<String>,
    pub minecraft_protocol: i32,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum HealthCheckMode {
    #[default]
    Tcp,
    MinecraftStatus,
}

#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum BackendHealthState {
    #[default]
    Unknown,
    Healthy,
    Unhealthy,
}

impl<'de> Deserialize<'de> for ProxyProtocolVersion {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Value {
            Enabled(bool),
            Version(String),
        }

        match Value::deserialize(deserializer)? {
            Value::Enabled(false) => Ok(Self::Off),
            Value::Enabled(true) => Ok(Self::V1),
            Value::Version(version) => match version.trim().to_ascii_lowercase().as_str() {
                "off" | "none" | "disabled" => Ok(Self::Off),
                "v1" => Ok(Self::V1),
                "v2" => Ok(Self::V2),
                _ => Err(serde::de::Error::custom(
                    "proxy_protocol 只支持 off、v1、v2 或布尔值",
                )),
            },
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct StatusResponseConfig {
    pub motd: Option<String>,
    pub version_name: Option<String>,
    pub protocol: Option<i32>,
    pub online: Option<u32>,
    pub max: Option<u32>,
}

#[derive(Clone, Debug)]
pub(crate) struct CachedStatus {
    pub expires_at: Instant,
    pub response_json: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct BackendHealthSnapshot {
    pub address: String,
    pub health: BackendHealthState,
    pub last_checked_secs_ago: Option<u64>,
    pub health_check_latency_ms: Option<u64>,
    pub consecutive_health_successes: u64,
    pub consecutive_health_failures: u64,
    pub health_check_successes: u64,
    pub health_check_failures: u64,
    pub active_connections: u64,
    pub successful_connections: u64,
    pub failed_attempts: u64,
    pub connect_latency_ms: Option<u64>,
}

#[derive(Debug)]
pub(crate) struct BackendPoolState {
    cursor: AtomicU64,
    backends: Vec<BackendRuntimeState>,
}

#[derive(Debug)]
struct BackendRuntimeState {
    address: String,
    health: AtomicU8,
    health_probe_in_flight: AtomicBool,
    last_health_check_unix_ms: AtomicU64,
    health_check_latency_micros: AtomicU64,
    consecutive_health_successes: AtomicU64,
    consecutive_health_failures: AtomicU64,
    health_check_successes: AtomicU64,
    health_check_failures: AtomicU64,
    active_connections: AtomicU64,
    successful_connections: AtomicU64,
    failed_attempts: AtomicU64,
    connect_latency_micros: AtomicU64,
}

pub(crate) struct BackendConnectionGuard {
    backend: Arc<BackendPoolState>,
    index: usize,
}

pub(crate) struct HealthProbeTarget {
    pool: Arc<BackendPoolState>,
    index: usize,
    pub(crate) address: String,
    pub(crate) timeout: Duration,
    pub(crate) mode: HealthCheckMode,
    pub(crate) minecraft_host: String,
    pub(crate) minecraft_protocol: i32,
    pub(crate) proxy_protocol: ProxyProtocolVersion,
    unhealthy_threshold: u32,
    healthy_threshold: u32,
    completed: bool,
}

#[derive(Clone, Debug)]
pub struct ForwardConfig {
    pub listen: SocketAddr,
    pub routes: Vec<RuleConfig>,
    pub connect_timeout_ms: u64,
    pub handshake_timeout_ms: u64,
    pub shutdown_grace_secs: u64,
    pub max_connections: usize,
    pub copy_buffer_bytes: usize,
    pub socket_buffer_bytes: usize,
    pub listen_backlog: i32,
    pub tcp_nodelay: bool,
    pub reuse_port: bool,
    pub stats_interval_secs: u64,
    pub(crate) status_cache: Arc<Mutex<HashMap<String, CachedStatus>>>,
    pub(crate) backend_pools: Arc<HashMap<String, Arc<BackendPoolState>>>,
    /// 原始后端地址到 ViaLite 回环监听地址的动态映射。
    pub(crate) via_dial_targets: Arc<tokio::sync::RwLock<HashMap<String, String>>>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            admin: AdminConfig::default(),
            crossplay: CrossplayConfig::default(),
            via: ViaLiteConfig::default(),
            settings: GlobalSettings::default(),
            rules: vec![RuleConfig::default()],
        }
    }
}

impl Default for ViaLiteConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            binary_path: None,
            runtime_dir: "/run/mc-proxy/vialite".to_string(),
            gate_protocol: "auto".to_string(),
            backend_version: "auto".to_string(),
        }
    }
}

impl Default for AdminConfig {
    fn default() -> Self {
        Self {
            listen: "127.0.0.1:18080"
                .parse()
                .expect("default admin address is valid"),
        }
    }
}

impl Default for GlobalSettings {
    fn default() -> Self {
        Self {
            listen: "0.0.0.0:25565"
                .parse()
                .expect("default proxy address is valid"),
            proxy_enabled: true,
            max_connections: 10_000,
            connect_timeout_ms: 5_000,
            handshake_timeout_ms: 5_000,
            shutdown_grace_secs: 30,
            copy_buffer_bytes: 32 * 1024,
            socket_buffer_bytes: 1024 * 1024,
            listen_backlog: 4096,
            tcp_nodelay: true,
            reuse_port: false,
            stats_interval_secs: 10,
        }
    }
}

impl Default for CrossplayConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: CrossplayProvider::External,
            bedrock_listen: "0.0.0.0:19132"
                .parse()
                .expect("default Bedrock address is valid"),
            java_address: "bedrock.example.com".to_string(),
            java_port: 25565,
            auth_type: CrossplayAuthType::Online,
            geyserlite: GeyserLiteConfig::default(),
        }
    }
}

impl Default for GeyserLiteConfig {
    fn default() -> Self {
        Self {
            mode: GeyserLiteMode::Embedded,
            library_path: None,
            binary_path: None,
            offline: false,
            motd_line1: "YvLink".to_string(),
            motd_line2: "Bedrock via GeyserLite".to_string(),
            floodgate_key: None,
        }
    }
}

impl Default for RuleConfig {
    fn default() -> Self {
        Self {
            id: "default".to_string(),
            name: "默认线路".to_string(),
            host: vec!["*".to_string()],
            backend: vec!["127.0.0.1:25566".to_string()],
            strategy: LoadBalancingStrategy::Sequential,
            proxy_protocol: ProxyProtocolVersion::Off,
            health_check: HealthCheckConfig::default(),
            modify_virtual_host: false,
            status: None,
            whitelist_enabled: false,
            whitelist: Vec::new(),
            whitelist_message: default_whitelist_message(),
            crossplay_enabled: false,
            enabled: true,
        }
    }
}

impl Default for HealthCheckConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: HealthCheckMode::Tcp,
            interval_secs: 30,
            timeout_ms: 2_000,
            unhealthy_threshold: 3,
            healthy_threshold: 2,
            minecraft_host: None,
            minecraft_protocol: 769,
        }
    }
}

impl Default for StatusConfig {
    fn default() -> Self {
        Self {
            mode: StatusMode::Custom,
            cache_ttl_secs: 10,
            motd: Some("§aMinecraft Server".to_string()),
            version_name: Some("MC Relay".to_string()),
            protocol: None,
            online: Some(0),
            max: Some(100),
            fallback: None,
        }
    }
}

fn default_status_cache_ttl_secs() -> i64 {
    10
}

impl AppConfig {
    pub fn load(explicit_path: Option<&Path>) -> Result<(Self, PathBuf)> {
        let selected = explicit_path
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("config.toml"));
        let config = if selected.exists() {
            let source = fs::read_to_string(&selected)
                .with_context(|| format!("无法读取配置文件 {}", selected.display()))?;
            toml::from_str(&source)
                .with_context(|| format!("无法解析配置文件 {}", selected.display()))?
        } else if explicit_path.is_some() {
            bail!("配置文件不存在: {}", selected.display());
        } else {
            Self::default()
        };
        config.validate()?;
        Ok((config, selected))
    }

    pub fn validate(&self) -> Result<()> {
        self.settings.validate()?;
        self.crossplay.validate()?;
        self.via.validate()?;
        if !self.admin.listen.ip().is_loopback() {
            bail!("admin.listen 必须监听回环地址，公网访问应通过 Nginx 反向代理");
        }
        if self.rules.is_empty() {
            bail!("至少需要保留一条 host -> backend 路由");
        }

        let mut ids = HashSet::new();
        let mut hosts = HashSet::new();
        for rule in &self.rules {
            rule.validate()?;
            if self.via.enabled && rule.proxy_protocol != ProxyProtocolVersion::Off {
                bail!(
                    "启用 via 时 rules.proxy_protocol 必须为 off：ViaLite 接收的是 Minecraft 握手，不接受 PROXY Protocol 头"
                );
            }
            if !ids.insert(rule.id.as_str()) {
                bail!("规则 id 重复: {}", rule.id);
            }
            if rule.enabled {
                for host in &rule.host {
                    let normalized = normalize_host_pattern(host)?;
                    if !hosts.insert(normalized.clone()) {
                        bail!("已启用的 host 匹配规则重复: {normalized}");
                    }
                }
            }
        }
        if self.settings.proxy_enabled && !self.rules.iter().any(|rule| rule.enabled) {
            bail!("代理启用时至少需要一条已启用的 host 路由");
        }
        if self.crossplay.enabled {
            if !self.settings.proxy_enabled {
                bail!("启用 Crossplay 前必须启用 Minecraft Java 入口");
            }
            if self.crossplay.java_port != self.settings.listen.port() {
                bail!(
                    "crossplay.java_port 必须与 settings.listen 端口一致，当前应为 {}",
                    self.settings.listen.port()
                );
            }
            if !self
                .rules
                .iter()
                .filter(|rule| rule.enabled && rule.crossplay_enabled)
                .any(|rule| {
                    rule.host
                        .iter()
                        .any(|pattern| host_matches(pattern, &self.crossplay.java_address))
                })
            {
                bail!(
                    "crossplay.java_address 未匹配任何已启用且允许 Crossplay 的路由: {}",
                    self.crossplay.java_address
                );
            }
        }
        Ok(())
    }

    pub fn persist(&self, path: &Path) -> Result<()> {
        self.validate()?;
        let source = toml::to_string_pretty(self).context("无法序列化配置")?;
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)
            .with_context(|| format!("无法创建配置目录 {}", parent.display()))?;
        let temporary = path.with_extension("toml.tmp");
        fs::write(&temporary, source)
            .with_context(|| format!("无法写入临时配置 {}", temporary.display()))?;
        fs::rename(&temporary, path)
            .with_context(|| format!("无法原子替换配置 {}", path.display()))?;
        Ok(())
    }
}

impl ViaLiteConfig {
    pub fn validate(&self) -> Result<()> {
        if self.runtime_dir.trim().is_empty() {
            bail!("via.runtime_dir 不能为空");
        }
        if self.runtime_dir.len() > 4096 {
            bail!("via.runtime_dir 过长");
        }
        for (name, value) in [
            ("via.gate_protocol", self.gate_protocol.as_str()),
            ("via.backend_version", self.backend_version.as_str()),
        ] {
            if value.trim().is_empty() || value.len() > 64 || value.chars().any(char::is_whitespace)
            {
                bail!("{name} 必须是长度不超过 64 的非空协议版本标识");
            }
        }
        if self.enabled {
            let path = self
                .binary_path
                .as_deref()
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .ok_or_else(|| {
                    anyhow!("启用 via 时必须设置 via.binary_path（由部署脚本校验和下载 ViaLite）")
                })?;
            if !Path::new(path).is_absolute() {
                bail!("via.binary_path 必须使用绝对路径");
            }
        }
        Ok(())
    }
}

impl CrossplayConfig {
    pub fn validate(&self) -> Result<()> {
        if self.bedrock_listen.port() == 0 {
            bail!("crossplay.bedrock_listen 端口不能为 0");
        }
        let java_address = self.java_address.trim();
        if java_address.is_empty()
            || java_address.len() > 253
            || java_address.chars().any(char::is_whitespace)
            || java_address.contains('\0')
        {
            bail!("crossplay.java_address 必须是有效的 Java 目标主机或 IP");
        }
        if self.java_port == 0 {
            bail!("crossplay.java_port 不能为 0");
        }
        if self.provider == CrossplayProvider::GeyserLite {
            let geyserlite = &self.geyserlite;
            match geyserlite.mode {
                GeyserLiteMode::Embedded => {
                    if geyserlite.binary_path.is_some() {
                        bail!("crossplay.geyserlite.binary_path 仅用于 subprocess 模式");
                    }
                }
                GeyserLiteMode::Subprocess => {
                    if geyserlite.library_path.is_some() {
                        bail!("crossplay.geyserlite.library_path 仅用于 embedded 模式");
                    }
                }
            }
            if let Some(key) = geyserlite.floodgate_key.as_deref() {
                let key = key.trim();
                if key.len() != 32 || !key.chars().all(|character| character.is_ascii_hexdigit()) {
                    bail!(
                        "crossplay.geyserlite.floodgate_key 必须是 16 字节密钥的 32 位十六进制字符串"
                    );
                }
            }
            if self.auth_type == CrossplayAuthType::Floodgate && geyserlite.floodgate_key.is_none()
            {
                bail!(
                    "provider = \"geyserlite\" 且 auth_type = \"floodgate\" 时必须提供 \
                     crossplay.geyserlite.floodgate_key"
                );
            }
        }
        Ok(())
    }
}

impl GlobalSettings {
    pub fn validate(&self) -> Result<()> {
        if self.listen.port() == 0 {
            bail!("settings.listen 端口不能为 0");
        }
        if self.max_connections == 0 || self.max_connections > 1_000_000 {
            bail!("max_connections 必须在 1..=1000000 之间");
        }
        if self.connect_timeout_ms == 0 || self.handshake_timeout_ms == 0 {
            bail!("连接和握手超时必须大于 0");
        }
        if !(1..=300).contains(&self.shutdown_grace_secs) {
            bail!("shutdown_grace_secs 必须在 1..=300 之间");
        }
        if !(MIN_COPY_BUFFER..=MAX_COPY_BUFFER).contains(&self.copy_buffer_bytes) {
            bail!("copy_buffer_bytes 必须在 {MIN_COPY_BUFFER}..={MAX_COPY_BUFFER} 之间");
        }
        if self.socket_buffer_bytes > 16 * 1024 * 1024 {
            bail!("socket_buffer_bytes 不能超过 16777216");
        }
        if !(1..=65_535).contains(&self.listen_backlog) {
            bail!("listen_backlog 必须在 1..=65535 之间");
        }
        if self.stats_interval_secs == 0 {
            bail!("stats_interval_secs 必须大于 0");
        }
        if self.reuse_port && !cfg!(any(target_os = "linux", target_os = "android")) {
            bail!("reuse_port 仅支持 Linux 或 Android");
        }
        Ok(())
    }
}

impl RuleConfig {
    pub fn validate(&self) -> Result<()> {
        if self.id.is_empty()
            || self.id.len() > 32
            || !self
                .id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        {
            bail!("规则 id 只能包含 1 到 32 个字母、数字、短横线或下划线");
        }
        let name = self.name.trim();
        if name.is_empty() || name.chars().count() > 64 {
            bail!("规则名称长度必须在 1..=64 个字符之间");
        }
        if self.host.is_empty() {
            bail!("规则 {} 至少需要一个 host", self.id);
        }
        for host in &self.host {
            normalize_host_pattern(host)?;
        }
        if self.host.iter().any(|host| host.trim() == "*") && self.host.len() != 1 {
            bail!("host = \"*\" 必须单独作为一条兜底规则");
        }
        if self.backend.is_empty() {
            bail!("规则 {} 至少需要一个 backend", self.id);
        }
        if self.backend.len() > 128 {
            bail!("规则 {} 的 backend 不能超过 128 个", self.id);
        }
        let mut backends = HashSet::new();
        for backend in &self.backend {
            validate_backend(backend)?;
            if !backends.insert(backend.trim().to_ascii_lowercase()) {
                bail!("规则 {} 的 backend 重复: {backend}", self.id);
            }
        }
        self.health_check.validate()?;
        if let Some(status) = &self.status {
            status.validate()?;
        }
        if self.whitelist.len() > 10_000 {
            bail!("规则 {} 的白名单不能超过 10000 个玩家", self.id);
        }
        let mut whitelist = HashSet::new();
        for player in &self.whitelist {
            let normalized = normalize_player_name(player)?;
            if !whitelist.insert(normalized.clone()) {
                bail!("规则 {} 的白名单玩家重复: {normalized}", self.id);
            }
        }
        if self.whitelist_enabled {
            let message = self.whitelist_message.trim();
            if message.is_empty() || message.chars().count() > 1024 {
                bail!("白名单拒绝消息长度必须在 1..=1024 个字符之间");
            }
        }
        Ok(())
    }
}

impl HealthCheckConfig {
    pub fn validate(&self) -> Result<()> {
        if !(1..=86_400).contains(&self.interval_secs) {
            bail!("健康检查间隔必须在 1..=86400 秒之间");
        }
        if !(100..=60_000).contains(&self.timeout_ms) {
            bail!("健康检查超时必须在 100..=60000 毫秒之间");
        }
        if self.timeout_ms > self.interval_secs.saturating_mul(1_000) {
            bail!("健康检查超时不能大于检查间隔");
        }
        if !(1..=100).contains(&self.unhealthy_threshold)
            || !(1..=100).contains(&self.healthy_threshold)
        {
            bail!("健康检查失败与恢复阈值必须在 1..=100 之间");
        }
        if let Some(host) = &self.minecraft_host {
            let host = host.trim();
            if host.is_empty()
                || host.len() > 255
                || host.contains('*')
                || host.contains('\0')
                || host.chars().any(char::is_whitespace)
            {
                bail!("Minecraft 健康检查 Host 必须是 1..=255 字节且不含通配符、空白或 NUL");
            }
        }
        if self.minecraft_protocol < 0 {
            bail!("Minecraft 健康检查协议号不能小于 0");
        }
        Ok(())
    }
}

impl StatusConfig {
    pub fn validate(&self) -> Result<()> {
        if !(-1..=86_400).contains(&self.cache_ttl_secs) {
            bail!("状态缓存 TTL 必须在 -1..=86400 秒之间，-1 表示禁用");
        }
        validate_status_response(
            self.motd.as_deref(),
            self.version_name.as_deref(),
            self.online,
            self.max,
        )?;
        if let Some(fallback) = &self.fallback {
            validate_status_response(
                fallback.motd.as_deref(),
                fallback.version_name.as_deref(),
                fallback.online,
                fallback.max,
            )?;
        }
        Ok(())
    }
}

fn validate_status_response(
    motd: Option<&str>,
    version_name: Option<&str>,
    online: Option<u32>,
    max: Option<u32>,
) -> Result<()> {
    if let Some(motd) = motd
        && (motd.trim().is_empty() || motd.chars().count() > 2048)
    {
        bail!("自定义 MOTD 长度必须在 1..=2048 个字符之间");
    }
    if let Some(version_name) = version_name
        && (version_name.trim().is_empty() || version_name.chars().count() > 64)
    {
        bail!("状态版本名称长度必须在 1..=64 个字符之间");
    }
    if online.is_some_and(|value| value > 1_000_000) || max.is_some_and(|value| value > 1_000_000) {
        bail!("状态玩家数必须在 0..=1000000 之间");
    }
    Ok(())
}

impl ForwardConfig {
    pub fn from_app(config: &AppConfig) -> Self {
        let settings = &config.settings;
        let routes: Vec<_> = config
            .rules
            .iter()
            .filter(|rule| rule.enabled)
            .cloned()
            .collect();
        let backend_pools = routes
            .iter()
            .map(|route| {
                (
                    route.id.clone(),
                    Arc::new(BackendPoolState::new(&route.backend)),
                )
            })
            .collect();
        Self {
            listen: settings.listen,
            routes,
            connect_timeout_ms: settings.connect_timeout_ms,
            handshake_timeout_ms: settings.handshake_timeout_ms,
            shutdown_grace_secs: settings.shutdown_grace_secs,
            max_connections: settings.max_connections,
            copy_buffer_bytes: settings.copy_buffer_bytes,
            socket_buffer_bytes: settings.socket_buffer_bytes,
            listen_backlog: settings.listen_backlog,
            tcp_nodelay: settings.tcp_nodelay,
            reuse_port: settings.reuse_port,
            stats_interval_secs: settings.stats_interval_secs,
            status_cache: Arc::new(Mutex::new(HashMap::new())),
            backend_pools: Arc::new(backend_pools),
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

    pub fn connect_timeout(&self) -> Duration {
        Duration::from_millis(self.connect_timeout_ms)
    }

    pub fn handshake_timeout(&self) -> Duration {
        Duration::from_millis(self.handshake_timeout_ms)
    }

    pub fn shutdown_grace(&self) -> Duration {
        Duration::from_secs(self.shutdown_grace_secs)
    }

    pub fn stats_interval(&self) -> Duration {
        Duration::from_secs(self.stats_interval_secs)
    }

    pub fn backend_health(&self, route_id: &str) -> Vec<BackendHealthSnapshot> {
        self.backend_pools
            .get(route_id)
            .map_or_else(Vec::new, |pool| pool.snapshot())
    }

    pub(crate) async fn via_dial_address(&self, backend: &str) -> String {
        self.via_dial_targets
            .read()
            .await
            .get(backend)
            .cloned()
            .unwrap_or_else(|| backend.to_string())
    }

    pub(crate) fn claim_due_health_probes(&self, limit: usize) -> Vec<HealthProbeTarget> {
        let now = unix_millis();
        self.routes
            .iter()
            .filter(|route| route.health_check.enabled)
            .flat_map(|route| {
                self.backend_pools
                    .get(&route.id)
                    .into_iter()
                    .flat_map(move |pool| {
                        pool.claim_due_health_probes(now, route, Arc::clone(pool))
                    })
            })
            .take(limit)
            .collect()
    }
}

impl BackendPoolState {
    fn new(backends: &[String]) -> Self {
        Self {
            cursor: AtomicU64::new(0),
            backends: backends
                .iter()
                .map(|address| BackendRuntimeState {
                    address: address.clone(),
                    health: AtomicU8::new(0),
                    health_probe_in_flight: AtomicBool::new(false),
                    last_health_check_unix_ms: AtomicU64::new(0),
                    health_check_latency_micros: AtomicU64::new(0),
                    consecutive_health_successes: AtomicU64::new(0),
                    consecutive_health_failures: AtomicU64::new(0),
                    health_check_successes: AtomicU64::new(0),
                    health_check_failures: AtomicU64::new(0),
                    active_connections: AtomicU64::new(0),
                    successful_connections: AtomicU64::new(0),
                    failed_attempts: AtomicU64::new(0),
                    connect_latency_micros: AtomicU64::new(0),
                })
                .collect(),
        }
    }

    pub(crate) fn candidate_indices(&self, strategy: LoadBalancingStrategy) -> Vec<usize> {
        let mut indices: Vec<_> = (0..self.backends.len()).collect();
        match strategy {
            LoadBalancingStrategy::Sequential => {}
            LoadBalancingStrategy::Random => {
                let mut value = self
                    .cursor
                    .fetch_add(0x9e37_79b9_7f4a_7c15, Ordering::Relaxed)
                    .wrapping_add(0x9e37_79b9_7f4a_7c15);
                for index in (1..indices.len()).rev() {
                    value ^= value >> 12;
                    value ^= value << 25;
                    value ^= value >> 27;
                    let selected =
                        (value.wrapping_mul(0x2545_f491_4f6c_dd1d) % (index as u64 + 1)) as usize;
                    indices.swap(index, selected);
                }
            }
            LoadBalancingStrategy::RoundRobin => {
                if !indices.is_empty() {
                    let start =
                        self.cursor.fetch_add(1, Ordering::Relaxed) as usize % indices.len();
                    indices.rotate_left(start);
                }
            }
            LoadBalancingStrategy::LeastConnections => {
                indices.sort_by_key(|index| {
                    (
                        self.backends[*index]
                            .active_connections
                            .load(Ordering::Relaxed),
                        *index,
                    )
                });
            }
            LoadBalancingStrategy::LowestLatency => {
                indices.sort_by_key(|index| {
                    let latency = self.backends[*index]
                        .connect_latency_micros
                        .load(Ordering::Relaxed);
                    (latency == 0, latency, *index)
                });
            }
        }
        indices.sort_by_key(|index| {
            self.backends[*index].health.load(Ordering::Acquire)
                == BackendHealthState::Unhealthy.as_u8()
        });
        indices
    }

    pub(crate) fn address(&self, index: usize) -> &str {
        &self.backends[index].address
    }

    pub(crate) fn failed(&self, index: usize) {
        self.backends[index]
            .failed_attempts
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn connected(
        self: &Arc<Self>,
        index: usize,
        latency: Duration,
    ) -> BackendConnectionGuard {
        let backend = &self.backends[index];
        backend.active_connections.fetch_add(1, Ordering::Relaxed);
        backend
            .successful_connections
            .fetch_add(1, Ordering::Relaxed);
        let measured = latency.as_micros().max(1).min(u128::from(u64::MAX)) as u64;
        let previous = backend.connect_latency_micros.load(Ordering::Relaxed);
        let smoothed = if previous == 0 {
            measured
        } else {
            previous.saturating_mul(7).saturating_add(measured) / 8
        };
        backend
            .connect_latency_micros
            .store(smoothed.max(1), Ordering::Relaxed);
        BackendConnectionGuard {
            backend: Arc::clone(self),
            index,
        }
    }

    fn snapshot(&self) -> Vec<BackendHealthSnapshot> {
        let now = unix_millis();
        self.backends
            .iter()
            .map(|backend| {
                let latency = backend.connect_latency_micros.load(Ordering::Relaxed);
                let health_latency = backend.health_check_latency_micros.load(Ordering::Relaxed);
                let last_checked = backend.last_health_check_unix_ms.load(Ordering::Acquire);
                BackendHealthSnapshot {
                    address: backend.address.clone(),
                    health: BackendHealthState::from_u8(backend.health.load(Ordering::Acquire)),
                    last_checked_secs_ago: (last_checked != 0)
                        .then(|| now.saturating_sub(last_checked) / 1_000),
                    health_check_latency_ms: (health_latency != 0)
                        .then_some(health_latency.div_ceil(1_000)),
                    consecutive_health_successes: backend
                        .consecutive_health_successes
                        .load(Ordering::Relaxed),
                    consecutive_health_failures: backend
                        .consecutive_health_failures
                        .load(Ordering::Relaxed),
                    health_check_successes: backend.health_check_successes.load(Ordering::Relaxed),
                    health_check_failures: backend.health_check_failures.load(Ordering::Relaxed),
                    active_connections: backend.active_connections.load(Ordering::Relaxed),
                    successful_connections: backend.successful_connections.load(Ordering::Relaxed),
                    failed_attempts: backend.failed_attempts.load(Ordering::Relaxed),
                    connect_latency_ms: (latency != 0).then_some(latency.div_ceil(1000)),
                }
            })
            .collect()
    }

    fn claim_due_health_probes(
        &self,
        now: u64,
        route: &RuleConfig,
        pool: Arc<Self>,
    ) -> Vec<HealthProbeTarget> {
        let config = &route.health_check;
        let interval_ms = config.interval_secs.saturating_mul(1_000);
        self.backends
            .iter()
            .enumerate()
            .filter_map(|(index, backend)| {
                let last_checked = backend.last_health_check_unix_ms.load(Ordering::Acquire);
                if last_checked != 0 && now.saturating_sub(last_checked) < interval_ms {
                    return None;
                }
                backend
                    .health_probe_in_flight
                    .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                    .ok()?;
                Some(HealthProbeTarget {
                    pool: Arc::clone(&pool),
                    index,
                    address: backend.address.clone(),
                    timeout: Duration::from_millis(config.timeout_ms),
                    mode: config.mode,
                    minecraft_host: health_probe_host(route, &backend.address),
                    minecraft_protocol: config.minecraft_protocol,
                    proxy_protocol: route.proxy_protocol,
                    unhealthy_threshold: config.unhealthy_threshold,
                    healthy_threshold: config.healthy_threshold,
                    completed: false,
                })
            })
            .collect()
    }
}

fn health_probe_host(route: &RuleConfig, backend: &str) -> String {
    if let Some(host) = route.health_check.minecraft_host.as_deref() {
        return host.trim().trim_end_matches('.').to_ascii_lowercase();
    }
    if !route.modify_virtual_host
        && let Some(host) = route
            .host
            .iter()
            .map(|host| host.trim())
            .find(|host| !host.contains('*') && !host.contains('?'))
    {
        return host.trim_end_matches('.').to_ascii_lowercase();
    }
    backend_host(backend).to_ascii_lowercase()
}

fn backend_host(backend: &str) -> &str {
    let host = backend.rsplit_once(':').map_or(backend, |(host, _)| host);
    host.strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(host)
}

impl BackendHealthState {
    const fn as_u8(self) -> u8 {
        match self {
            Self::Unknown => 0,
            Self::Healthy => 1,
            Self::Unhealthy => 2,
        }
    }

    const fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Healthy,
            2 => Self::Unhealthy,
            _ => Self::Unknown,
        }
    }
}

impl HealthProbeTarget {
    pub(crate) fn complete(mut self, result: std::io::Result<Duration>) {
        let backend = &self.pool.backends[self.index];
        match result {
            Ok(latency) => {
                let measured = latency.as_micros().max(1).min(u128::from(u64::MAX)) as u64;
                backend
                    .health_check_latency_micros
                    .store(measured, Ordering::Relaxed);
                backend
                    .health_check_successes
                    .fetch_add(1, Ordering::Relaxed);
                backend
                    .consecutive_health_failures
                    .store(0, Ordering::Relaxed);
                let successes = backend
                    .consecutive_health_successes
                    .fetch_add(1, Ordering::Relaxed)
                    .saturating_add(1);
                if successes >= u64::from(self.healthy_threshold) {
                    backend
                        .health
                        .store(BackendHealthState::Healthy.as_u8(), Ordering::Release);
                }
                let previous = backend.connect_latency_micros.load(Ordering::Relaxed);
                let smoothed = if previous == 0 {
                    measured
                } else {
                    previous.saturating_mul(7).saturating_add(measured) / 8
                };
                backend
                    .connect_latency_micros
                    .store(smoothed.max(1), Ordering::Relaxed);
            }
            Err(_) => {
                backend
                    .health_check_failures
                    .fetch_add(1, Ordering::Relaxed);
                backend
                    .consecutive_health_successes
                    .store(0, Ordering::Relaxed);
                let failures = backend
                    .consecutive_health_failures
                    .fetch_add(1, Ordering::Relaxed)
                    .saturating_add(1);
                if failures >= u64::from(self.unhealthy_threshold) {
                    backend
                        .health
                        .store(BackendHealthState::Unhealthy.as_u8(), Ordering::Release);
                }
            }
        }
        backend
            .last_health_check_unix_ms
            .store(unix_millis(), Ordering::Release);
        backend
            .health_probe_in_flight
            .store(false, Ordering::Release);
        self.completed = true;
    }
}

impl Drop for HealthProbeTarget {
    fn drop(&mut self) {
        if !self.completed {
            self.pool.backends[self.index]
                .health_probe_in_flight
                .store(false, Ordering::Release);
        }
    }
}

fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

impl Drop for BackendConnectionGuard {
    fn drop(&mut self) {
        self.backend.backends[self.index]
            .active_connections
            .fetch_sub(1, Ordering::Relaxed);
    }
}

pub fn normalize_host_pattern(host: &str) -> Result<String> {
    let normalized = host.trim().trim_end_matches('.').to_ascii_lowercase();
    if normalized.is_empty() || normalized.len() > 253 || normalized.contains(':') {
        bail!("无效的 host 匹配规则: {host}");
    }
    if normalized == "*" {
        return Ok(normalized);
    }
    for label in normalized.split('.') {
        if label.is_empty()
            || label.len() > 63
            || label.starts_with('-')
            || label.ends_with('-')
            || !label.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'*' || byte == b'?'
            })
        {
            bail!("无效的 host 匹配规则: {host}");
        }
    }
    Ok(normalized)
}

pub(crate) fn host_matches(pattern: &str, hostname: &str) -> bool {
    let pattern = pattern.trim().trim_end_matches('.').to_ascii_lowercase();
    let hostname = hostname.trim_end_matches('.').to_ascii_lowercase();
    let pattern = pattern.as_bytes();
    let hostname = hostname.as_bytes();
    let (mut pattern_index, mut host_index) = (0, 0);
    let mut star = None;
    let mut retry_host = 0;

    while host_index < hostname.len() {
        if pattern_index < pattern.len()
            && (pattern[pattern_index] == b'?' || pattern[pattern_index] == hostname[host_index])
        {
            pattern_index += 1;
            host_index += 1;
        } else if pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
            star = Some(pattern_index);
            pattern_index += 1;
            retry_host = host_index;
        } else if let Some(star_index) = star {
            pattern_index = star_index + 1;
            retry_host += 1;
            host_index = retry_host;
        } else {
            return false;
        }
    }
    while pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
        pattern_index += 1;
    }
    pattern_index == pattern.len()
}

pub fn validate_backend(backend: &str) -> Result<()> {
    let backend = backend.trim();
    let (host, port) = backend
        .rsplit_once(':')
        .ok_or_else(|| anyhow::anyhow!("backend 必须使用 主机:端口 格式: {backend}"))?;
    if host.is_empty() || host.contains(char::is_whitespace) {
        bail!("backend 主机无效: {backend}");
    }
    let port: u16 = port
        .parse()
        .with_context(|| format!("backend 端口无效: {backend}"))?;
    if port == 0 {
        bail!("backend 端口不能为 0");
    }
    Ok(())
}

pub fn normalize_player_name(player: &str) -> Result<String> {
    let player = player.trim();
    if player.is_empty()
        || player.len() > 16
        || !player
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        bail!("无效的 Minecraft 玩家名: {player}");
    }
    Ok(player.to_ascii_lowercase())
}

fn default_true() -> bool {
    true
}

fn default_whitelist_message() -> String {
    "§c你不在此服务器的白名单中。".to_string()
}

fn one_or_many<'de, D>(deserializer: D) -> std::result::Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Value {
        One(String),
        Many(Vec<String>),
    }
    Ok(match Value::deserialize(deserializer)? {
        Value::One(value) => vec![value],
        Value::Many(values) => values,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid() {
        AppConfig::default().validate().unwrap();
    }

    #[test]
    fn accepts_scalar_or_list_host() {
        let scalar = r#"id="a"
name="A"
host="*.example.com"
backend="backend.example.com:25565"
modify_virtual_host=true
"#;
        let list = r#"id="a"
name="A"
host=["example.com", "localhost"]
backend="127.0.0.1:25565"
modify_virtual_host=false
"#;
        assert_eq!(toml::from_str::<RuleConfig>(scalar).unwrap().host.len(), 1);
        assert_eq!(toml::from_str::<RuleConfig>(list).unwrap().host.len(), 2);
        assert_eq!(
            toml::from_str::<RuleConfig>(scalar).unwrap().backend,
            vec!["backend.example.com:25565"]
        );
        assert_eq!(
            toml::from_str::<RuleConfig>(
                r#"id="pool"
name="Pool"
host="pool.example.com"
backend=["10.0.0.1:25565", "10.0.0.2:25565"]
strategy="least-connections"
"#,
            )
            .unwrap()
            .backend
            .len(),
            2
        );
    }

    #[test]
    fn proxy_protocol_accepts_versions_and_gate_style_boolean() {
        let rule = |value: &str| {
            toml::from_str::<RuleConfig>(&format!(
                r#"id="proxy"
name="Proxy"
host="proxy.example.com"
backend="127.0.0.1:25565"
proxy_protocol={value}
"#,
            ))
            .unwrap()
            .proxy_protocol
        };
        assert_eq!(rule("\"off\""), ProxyProtocolVersion::Off);
        assert_eq!(rule("\"v1\""), ProxyProtocolVersion::V1);
        assert_eq!(rule("\"v2\""), ProxyProtocolVersion::V2);
        assert_eq!(rule("true"), ProxyProtocolVersion::V1);
        assert_eq!(rule("false"), ProxyProtocolVersion::Off);
    }

    #[test]
    fn validates_health_check_configuration() {
        let mut health = HealthCheckConfig::default();
        health.validate().unwrap();
        health.interval_secs = 1;
        health.timeout_ms = 1_001;
        assert!(health.validate().is_err());
        health.timeout_ms = 100;
        health.unhealthy_threshold = 0;
        assert!(health.validate().is_err());
        health.unhealthy_threshold = 1;
        health.mode = HealthCheckMode::MinecraftStatus;
        health.minecraft_host = Some("*.invalid.example.com".to_string());
        assert!(health.validate().is_err());
        health.minecraft_host = Some("probe.example.com".to_string());
        health.minecraft_protocol = -1;
        assert!(health.validate().is_err());
    }

    #[test]
    fn active_health_state_prioritizes_viable_backends_and_recovers() {
        let pool = Arc::new(BackendPoolState::new(&[
            "127.0.0.1:25561".to_string(),
            "127.0.0.1:25562".to_string(),
            "127.0.0.1:25563".to_string(),
        ]));
        let complete = |index, result| {
            HealthProbeTarget {
                pool: Arc::clone(&pool),
                index,
                address: pool.address(index).to_string(),
                timeout: Duration::from_secs(1),
                mode: HealthCheckMode::Tcp,
                minecraft_host: "localhost".to_string(),
                minecraft_protocol: 769,
                proxy_protocol: ProxyProtocolVersion::Off,
                unhealthy_threshold: 2,
                healthy_threshold: 2,
                completed: false,
            }
            .complete(result);
        };

        complete(0, Err(std::io::Error::other("offline")));
        assert_eq!(pool.snapshot()[0].health, BackendHealthState::Unknown);
        complete(0, Err(std::io::Error::other("offline")));
        complete(1, Err(std::io::Error::other("offline")));
        complete(1, Err(std::io::Error::other("offline")));
        complete(2, Ok(Duration::from_millis(5)));
        complete(2, Ok(Duration::from_millis(5)));
        assert_eq!(
            pool.candidate_indices(LoadBalancingStrategy::Sequential),
            vec![2, 0, 1]
        );

        complete(0, Ok(Duration::from_millis(10)));
        complete(0, Ok(Duration::from_millis(10)));
        assert_eq!(pool.snapshot()[0].health, BackendHealthState::Healthy);
        assert_eq!(
            pool.candidate_indices(LoadBalancingStrategy::Sequential),
            vec![0, 2, 1]
        );
    }

    #[test]
    fn derives_health_probe_host_from_route_or_backend() {
        let mut app = AppConfig::default();
        app.rules[0].host = vec!["play.example.com".to_string()];
        app.rules[0].backend = vec!["127.0.0.1:25565".to_string()];
        app.rules[0].health_check.enabled = true;
        app.rules[0].health_check.mode = HealthCheckMode::MinecraftStatus;
        let config = ForwardConfig::from_app(&app);
        let target = config.claim_due_health_probes(1).pop().unwrap();
        assert_eq!(target.minecraft_host, "play.example.com");

        app.rules[0].modify_virtual_host = true;
        let config = ForwardConfig::from_app(&app);
        let target = config.claim_due_health_probes(1).pop().unwrap();
        assert_eq!(target.minecraft_host, "127.0.0.1");

        app.rules[0].health_check.minecraft_host = Some("Health.Example.COM.".to_string());
        let config = ForwardConfig::from_app(&app);
        let target = config.claim_due_health_probes(1).pop().unwrap();
        assert_eq!(target.minecraft_host, "health.example.com");
    }

    #[test]
    fn health_probe_claim_prevents_duplicates_and_releases_on_drop() {
        let mut app = AppConfig::default();
        app.rules[0].health_check.enabled = true;
        let config = ForwardConfig::from_app(&app);
        let claimed = config.claim_due_health_probes(1);
        assert_eq!(claimed.len(), 1);
        assert!(config.claim_due_health_probes(1).is_empty());
        drop(claimed);
        assert_eq!(config.claim_due_health_probes(1).len(), 1);
    }

    #[test]
    fn backend_pool_strategies_produce_valid_candidate_orders() {
        let pool = Arc::new(BackendPoolState::new(&[
            "127.0.0.1:25561".to_string(),
            "127.0.0.1:25562".to_string(),
            "127.0.0.1:25563".to_string(),
        ]));
        assert_eq!(
            pool.candidate_indices(LoadBalancingStrategy::Sequential),
            vec![0, 1, 2]
        );
        assert_eq!(
            pool.candidate_indices(LoadBalancingStrategy::RoundRobin),
            vec![0, 1, 2]
        );
        assert_eq!(
            pool.candidate_indices(LoadBalancingStrategy::RoundRobin),
            vec![1, 2, 0]
        );
        let active = pool.connected(0, Duration::from_millis(20));
        assert_eq!(
            pool.candidate_indices(LoadBalancingStrategy::LeastConnections)[0],
            1
        );
        drop(active);
        let _second = pool.connected(1, Duration::from_millis(5));
        let _third = pool.connected(2, Duration::from_millis(40));
        assert_eq!(
            pool.candidate_indices(LoadBalancingStrategy::LowestLatency),
            vec![1, 0, 2]
        );
        let random = pool.candidate_indices(LoadBalancingStrategy::Random);
        assert_eq!(random.len(), 3);
        assert!(random.iter().all(|index| *index < 3));
    }

    #[test]
    fn crossplay_requires_the_java_ingress_and_a_matching_route() {
        let mut config = AppConfig::default();
        config.crossplay.enabled = true;
        config.crossplay.java_address = "example.com".to_string();
        config.rules[0].crossplay_enabled = true;
        assert!(config.validate().is_ok());

        config.crossplay.java_port = 25566;
        assert!(config.validate().is_err());
        config.crossplay.java_port = 25565;
        config.rules[0].host = vec!["play.example.com".to_string()];
        assert!(config.validate().is_err());
        config.rules[0].host = vec!["example.com".to_string()];
        config.rules[0].crossplay_enabled = false;
        assert!(config.validate().is_err());
        config.crossplay.enabled = false;
        assert!(config.validate().is_ok());
    }

    #[test]
    fn legacy_rule_without_crossplay_flag_defaults_to_disabled() {
        let rule: RuleConfig = toml::from_str(
            r#"
id = "legacy"
name = "旧路由"
host = "legacy.example.com"
backend = "127.0.0.1:25565"
"#,
        )
        .unwrap();
        assert!(!rule.crossplay_enabled);
    }

    #[test]
    fn validates_wildcards_and_catch_all() {
        assert_eq!(
            normalize_host_pattern("*.Play.Example.COM.").unwrap(),
            "*.play.example.com"
        );
        assert_eq!(normalize_host_pattern("*").unwrap(), "*");
        assert!(normalize_host_pattern("-bad.example.com").is_err());
    }

    #[test]
    fn rejects_duplicate_enabled_hosts() {
        let mut config = AppConfig::default();
        let mut duplicate = config.rules[0].clone();
        duplicate.id = "other".to_string();
        config.rules.push(duplicate);
        assert!(config.validate().is_err());
    }

    #[test]
    fn validates_custom_status_and_case_insensitive_whitelist() {
        let mut rule = RuleConfig {
            status: Some(StatusConfig::default()),
            whitelist_enabled: true,
            whitelist: vec!["Alice".to_string(), "bob_2".to_string()],
            ..RuleConfig::default()
        };
        rule.validate().unwrap();
        assert_eq!(normalize_player_name(" ALICE ").unwrap(), "alice");

        rule.whitelist.push("alice".to_string());
        assert!(rule.validate().is_err());
        rule.whitelist = vec!["bad-player!".to_string()];
        assert!(rule.validate().is_err());
    }

    #[test]
    fn backend_status_only_overrides_explicit_fields() {
        let status: StatusConfig = toml::from_str(
            r#"
mode = "backend"
cache_ttl_secs = 60
motd = "§a覆盖 MOTD"
"#,
        )
        .unwrap();

        assert_eq!(status.mode, StatusMode::Backend);
        assert_eq!(status.cache_ttl_secs, 60);
        assert_eq!(status.motd.as_deref(), Some("§a覆盖 MOTD"));
        assert_eq!(status.version_name, None);
        assert_eq!(status.protocol, None);
        assert_eq!(status.online, None);
        assert_eq!(status.max, None);
    }

    #[test]
    fn bundled_config_examples_are_valid() {
        for source in [
            include_str!("../config.example.toml"),
            include_str!("../deploy/config.production.toml"),
            include_str!("../tests/healthcheck.standalone.toml"),
            include_str!("../tests/status-healthcheck.standalone.toml"),
        ] {
            toml::from_str::<AppConfig>(source)
                .unwrap()
                .validate()
                .unwrap();
        }
    }

    #[test]
    fn legacy_crossplay_without_provider_parses_as_external() {
        let config: CrossplayConfig = toml::from_str(
            r#"
enabled = true
bedrock_listen = "0.0.0.0:19132"
java_address = "bedrock.example.com"
java_port = 25565
auth_type = "online"
"#,
        )
        .unwrap();
        assert_eq!(config.provider, CrossplayProvider::External);
        assert_eq!(config.geyserlite, GeyserLiteConfig::default());
        config.validate().unwrap();
    }

    #[test]
    fn geyserlite_floodgate_requires_hex_key() {
        let base = CrossplayConfig {
            provider: CrossplayProvider::GeyserLite,
            auth_type: CrossplayAuthType::Floodgate,
            ..CrossplayConfig::default()
        };
        assert!(base.validate().is_err());

        let invalid_key = CrossplayConfig {
            geyserlite: GeyserLiteConfig {
                floodgate_key: Some("zz112233445566778899aabbccddeeff".to_string()),
                ..GeyserLiteConfig::default()
            },
            ..base.clone()
        };
        assert!(invalid_key.validate().is_err());

        let valid = CrossplayConfig {
            geyserlite: GeyserLiteConfig {
                floodgate_key: Some("00112233445566778899aabbccddeeff".to_string()),
                ..GeyserLiteConfig::default()
            },
            ..base
        };
        valid.validate().unwrap();
    }

    #[test]
    fn geyserlite_mode_path_conflicts_are_rejected() {
        let embedded_with_binary = CrossplayConfig {
            provider: CrossplayProvider::GeyserLite,
            geyserlite: GeyserLiteConfig {
                mode: GeyserLiteMode::Embedded,
                binary_path: Some("/opt/geyserlite".to_string()),
                ..GeyserLiteConfig::default()
            },
            ..CrossplayConfig::default()
        };
        assert!(embedded_with_binary.validate().is_err());

        let subprocess_with_library = CrossplayConfig {
            provider: CrossplayProvider::GeyserLite,
            geyserlite: GeyserLiteConfig {
                mode: GeyserLiteMode::Subprocess,
                library_path: Some("/opt/libgeyserlite.so".to_string()),
                ..GeyserLiteConfig::default()
            },
            ..CrossplayConfig::default()
        };
        assert!(subprocess_with_library.validate().is_err());

        let valid = CrossplayConfig {
            provider: CrossplayProvider::GeyserLite,
            geyserlite: GeyserLiteConfig {
                mode: GeyserLiteMode::Subprocess,
                binary_path: Some("/opt/geyserlite".to_string()),
                offline: true,
                ..GeyserLiteConfig::default()
            },
            ..CrossplayConfig::default()
        };
        valid.validate().unwrap();
    }

    #[test]
    fn geyserlite_provider_roundtrips_with_product_name() {
        let config = CrossplayConfig {
            provider: CrossplayProvider::GeyserLite,
            ..CrossplayConfig::default()
        };
        let serialized = toml::to_string(&config).unwrap();
        assert!(serialized.contains("provider = \"geyserlite\""));
        let parsed: CrossplayConfig = toml::from_str(&serialized).unwrap();
        assert_eq!(parsed.provider, CrossplayProvider::GeyserLite);
    }

    #[test]
    fn via_requires_an_absolute_binary_and_disables_proxy_protocol() {
        let mut config = AppConfig::default();
        config.via.enabled = true;
        assert!(config.validate().is_err());

        config.via.binary_path = Some("vialite".to_string());
        assert!(config.validate().is_err());

        config.via.binary_path = Some("/opt/mc-proxy/vialite/vialite".to_string());
        assert!(config.validate().is_ok());
        config.rules[0].proxy_protocol = ProxyProtocolVersion::V2;
        assert!(config.validate().is_err());
    }
}
