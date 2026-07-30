use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use serde::Serialize;

#[derive(Debug, Default)]
pub struct Metrics {
    accepted_connections: AtomicU64,
    active_connections: AtomicU64,
    rejected_connections: AtomicU64,
    unmarked_handshakes: AtomicU64,
    legacy_forge_handshakes: AtomicU64,
    modern_forge_login_handshakes: AtomicU64,
    configuration_forge_handshakes: AtomicU64,
    proxy_protocol_v1_headers: AtomicU64,
    proxy_protocol_v2_headers: AtomicU64,
    health_check_successes: AtomicU64,
    health_check_failures: AtomicU64,
    whitelist_denials: AtomicU64,
    local_status_responses: AtomicU64,
    status_cache_hits: AtomicU64,
    status_fallbacks: AtomicU64,
    backend_attempt_failures: AtomicU64,
    backend_failovers: AtomicU64,
    backend_failures: AtomicU64,
    forwarding_failures: AtomicU64,
    upload_bytes: AtomicU64,
    download_bytes: AtomicU64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct MetricsSnapshot {
    pub accepted_connections: u64,
    pub active_connections: u64,
    pub rejected_connections: u64,
    pub unmarked_handshakes: u64,
    pub legacy_forge_handshakes: u64,
    pub modern_forge_login_handshakes: u64,
    pub configuration_forge_handshakes: u64,
    pub proxy_protocol_v1_headers: u64,
    pub proxy_protocol_v2_headers: u64,
    pub health_check_successes: u64,
    pub health_check_failures: u64,
    pub whitelist_denials: u64,
    pub local_status_responses: u64,
    pub status_cache_hits: u64,
    pub status_fallbacks: u64,
    pub backend_attempt_failures: u64,
    pub backend_failovers: u64,
    pub backend_failures: u64,
    pub forwarding_failures: u64,
    pub upload_bytes: u64,
    pub download_bytes: u64,
}

impl MetricsSnapshot {
    pub fn add_assign(&mut self, other: Self) {
        self.accepted_connections += other.accepted_connections;
        self.active_connections += other.active_connections;
        self.rejected_connections += other.rejected_connections;
        self.unmarked_handshakes += other.unmarked_handshakes;
        self.legacy_forge_handshakes += other.legacy_forge_handshakes;
        self.modern_forge_login_handshakes += other.modern_forge_login_handshakes;
        self.configuration_forge_handshakes += other.configuration_forge_handshakes;
        self.proxy_protocol_v1_headers += other.proxy_protocol_v1_headers;
        self.proxy_protocol_v2_headers += other.proxy_protocol_v2_headers;
        self.health_check_successes += other.health_check_successes;
        self.health_check_failures += other.health_check_failures;
        self.whitelist_denials += other.whitelist_denials;
        self.local_status_responses += other.local_status_responses;
        self.status_cache_hits += other.status_cache_hits;
        self.status_fallbacks += other.status_fallbacks;
        self.backend_attempt_failures += other.backend_attempt_failures;
        self.backend_failovers += other.backend_failovers;
        self.backend_failures += other.backend_failures;
        self.forwarding_failures += other.forwarding_failures;
        self.upload_bytes += other.upload_bytes;
        self.download_bytes += other.download_bytes;
    }
}

impl Metrics {
    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            accepted_connections: self.accepted_connections.load(Ordering::Relaxed),
            active_connections: self.active_connections.load(Ordering::Relaxed),
            rejected_connections: self.rejected_connections.load(Ordering::Relaxed),
            unmarked_handshakes: self.unmarked_handshakes.load(Ordering::Relaxed),
            legacy_forge_handshakes: self.legacy_forge_handshakes.load(Ordering::Relaxed),
            modern_forge_login_handshakes: self
                .modern_forge_login_handshakes
                .load(Ordering::Relaxed),
            configuration_forge_handshakes: self
                .configuration_forge_handshakes
                .load(Ordering::Relaxed),
            proxy_protocol_v1_headers: self.proxy_protocol_v1_headers.load(Ordering::Relaxed),
            proxy_protocol_v2_headers: self.proxy_protocol_v2_headers.load(Ordering::Relaxed),
            health_check_successes: self.health_check_successes.load(Ordering::Relaxed),
            health_check_failures: self.health_check_failures.load(Ordering::Relaxed),
            whitelist_denials: self.whitelist_denials.load(Ordering::Relaxed),
            local_status_responses: self.local_status_responses.load(Ordering::Relaxed),
            status_cache_hits: self.status_cache_hits.load(Ordering::Relaxed),
            status_fallbacks: self.status_fallbacks.load(Ordering::Relaxed),
            backend_attempt_failures: self.backend_attempt_failures.load(Ordering::Relaxed),
            backend_failovers: self.backend_failovers.load(Ordering::Relaxed),
            backend_failures: self.backend_failures.load(Ordering::Relaxed),
            forwarding_failures: self.forwarding_failures.load(Ordering::Relaxed),
            upload_bytes: self.upload_bytes.load(Ordering::Relaxed),
            download_bytes: self.download_bytes.load(Ordering::Relaxed),
        }
    }

    pub(crate) fn accepted(&self) {
        self.accepted_connections.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn rejected(&self) {
        self.rejected_connections.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn observed_handshake(&self, flavor: HandshakeFlavor) {
        let counter = match flavor {
            HandshakeFlavor::Unmarked => &self.unmarked_handshakes,
            HandshakeFlavor::LegacyForge => &self.legacy_forge_handshakes,
            HandshakeFlavor::ModernForgeLogin => &self.modern_forge_login_handshakes,
            HandshakeFlavor::ConfigurationForge => &self.configuration_forge_handshakes,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn proxy_protocol_header(&self, version: crate::ProxyProtocolVersion) {
        match version {
            crate::ProxyProtocolVersion::Off => {}
            crate::ProxyProtocolVersion::V1 => {
                self.proxy_protocol_v1_headers
                    .fetch_add(1, Ordering::Relaxed);
            }
            crate::ProxyProtocolVersion::V2 => {
                self.proxy_protocol_v2_headers
                    .fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    pub(crate) fn health_check_succeeded(&self) {
        self.health_check_successes.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn health_check_failed(&self) {
        self.health_check_failures.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn whitelist_denied(&self) {
        self.whitelist_denials.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn local_status_responded(&self) {
        self.local_status_responses.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn status_cache_hit(&self) {
        self.status_cache_hits.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn status_fallback(&self) {
        self.status_fallbacks.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn backend_attempt_failed(&self) {
        self.backend_attempt_failures
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn backend_failover(&self) {
        self.backend_failovers.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn backend_failed(&self) {
        self.backend_failures.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn forwarding_failed(&self) {
        self.forwarding_failures.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn uploaded(&self, bytes: u64) {
        self.upload_bytes.fetch_add(bytes, Ordering::Relaxed);
    }

    pub(crate) fn downloaded(&self, bytes: u64) {
        self.download_bytes.fetch_add(bytes, Ordering::Relaxed);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HandshakeFlavor {
    Unmarked,
    LegacyForge,
    ModernForgeLogin,
    ConfigurationForge,
}

pub(crate) struct ActiveConnectionGuard {
    metrics: Arc<Metrics>,
}

impl ActiveConnectionGuard {
    pub(crate) fn new(metrics: Arc<Metrics>) -> Self {
        metrics.active_connections.fetch_add(1, Ordering::Relaxed);
        Self { metrics }
    }
}

impl Drop for ActiveConnectionGuard {
    fn drop(&mut self) {
        self.metrics
            .active_connections
            .fetch_sub(1, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_guard_always_releases_counter() {
        let metrics = Arc::new(Metrics::default());
        {
            let _guard = ActiveConnectionGuard::new(Arc::clone(&metrics));
            assert_eq!(metrics.snapshot().active_connections, 1);
        }
        assert_eq!(metrics.snapshot().active_connections, 0);
    }

    #[test]
    fn snapshot_contains_live_bytes() {
        let metrics = Metrics::default();
        metrics.uploaded(12);
        metrics.downloaded(34);
        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.upload_bytes, 12);
        assert_eq!(snapshot.download_bytes, 34);
    }

    #[test]
    fn handshake_flavors_are_counted_separately() {
        let metrics = Metrics::default();
        metrics.observed_handshake(HandshakeFlavor::Unmarked);
        metrics.observed_handshake(HandshakeFlavor::LegacyForge);
        metrics.observed_handshake(HandshakeFlavor::ModernForgeLogin);
        metrics.observed_handshake(HandshakeFlavor::ModernForgeLogin);
        metrics.observed_handshake(HandshakeFlavor::ConfigurationForge);
        metrics.proxy_protocol_header(crate::ProxyProtocolVersion::V1);
        metrics.proxy_protocol_header(crate::ProxyProtocolVersion::V2);

        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.unmarked_handshakes, 1);
        assert_eq!(snapshot.legacy_forge_handshakes, 1);
        assert_eq!(snapshot.modern_forge_login_handshakes, 2);
        assert_eq!(snapshot.configuration_forge_handshakes, 1);
        assert_eq!(snapshot.proxy_protocol_v1_headers, 1);
        assert_eq!(snapshot.proxy_protocol_v2_headers, 1);
    }
}
