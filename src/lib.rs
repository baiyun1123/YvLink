pub mod api;
pub mod config;
pub mod crossplay;
pub mod listener;
pub mod manager;
pub mod metrics;
pub mod proxy;
pub mod server;
pub mod web;

pub use config::{
    AppConfig, BackendHealthSnapshot, BackendHealthState, CrossplayAuthType, CrossplayConfig,
    ForwardConfig, GlobalSettings, HealthCheckConfig, HealthCheckMode, LoadBalancingStrategy,
    ProxyProtocolVersion, RuleConfig, StatusConfig, StatusMode, StatusResponseConfig,
};
pub use crossplay::{CrossplayStatus, crossplay_status};
pub use listener::create_listener;
pub use manager::{RuleStatus, RuntimeManager, RuntimeStatus, validate_admin_token};
pub use metrics::{Metrics, MetricsSnapshot};
pub use proxy::{ProxyError, TransferReport, proxy_connection};
pub use server::serve;
