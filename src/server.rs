use std::{
    future::Future,
    io,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::Result;
use tokio::{
    net::{TcpListener, TcpStream},
    sync::{Semaphore, TryAcquireError},
    task::JoinSet,
    time::{MissedTickBehavior, interval, timeout},
};
use tracing::{debug, error, info, warn};

use crate::{
    ForwardConfig, HealthCheckMode, Metrics,
    metrics::ActiveConnectionGuard,
    proxy::{ProxyError, probe_minecraft_status, proxy_connection},
};

const MAX_CONCURRENT_HEALTH_CHECKS: usize = 64;

pub async fn serve<S>(
    listener: TcpListener,
    config: Arc<ForwardConfig>,
    metrics: Arc<Metrics>,
    shutdown: S,
) -> Result<()>
where
    S: Future<Output = ()>,
{
    let semaphore = Arc::new(Semaphore::new(config.max_connections));
    let mut tasks = JoinSet::new();
    let mut health_tasks = JoinSet::new();
    let mut stats_tick = interval(config.stats_interval());
    stats_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    stats_tick.tick().await;
    let mut health_tick = interval(Duration::from_secs(1));
    health_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            biased;

            () = &mut shutdown => {
                info!("收到退出信号，停止接受新连接");
                break;
            }
            task_result = tasks.join_next(), if !tasks.is_empty() => {
                report_task_result(task_result);
            }
            health_result = health_tasks.join_next(), if !health_tasks.is_empty() => {
                report_task_result(health_result);
            }
            _ = health_tick.tick() => {
                let available = MAX_CONCURRENT_HEALTH_CHECKS.saturating_sub(health_tasks.len());
                for target in config.claim_due_health_probes(available) {
                    let probe_metrics = Arc::clone(&metrics);
                    health_tasks.spawn(run_health_probe(target, probe_metrics));
                }
            }
            _ = stats_tick.tick() => {
                log_snapshot(&metrics);
            }
            accept_result = listener.accept() => {
                match accept_result {
                    Ok((client, client_addr)) => {
                        metrics.accepted();
                        let permit = match Arc::clone(&semaphore).try_acquire_owned() {
                            Ok(permit) => permit,
                            Err(TryAcquireError::NoPermits) => {
                                metrics.rejected();
                                debug!(%client_addr, "连接数已达上限，拒绝连接");
                                drop(client);
                                continue;
                            }
                            Err(TryAcquireError::Closed) => {
                                warn!("连接限制器已关闭");
                                drop(client);
                                break;
                            }
                        };

                        let task_config = Arc::clone(&config);
                        let task_metrics = Arc::clone(&metrics);
                        tasks.spawn(async move {
                            let _permit = permit;
                            let _active = ActiveConnectionGuard::new(Arc::clone(&task_metrics));
                            match proxy_connection(
                                client,
                                &task_config,
                                Arc::clone(&task_metrics),
                            ).await {
                                Ok(report) => {
                                    debug!(
                                        %client_addr,
                                        upload_bytes = report.upload_bytes,
                                        download_bytes = report.download_bytes,
                                        elapsed_ms = report.elapsed_millis,
                                        backend = %report.backend,
                                        route_id = %report.route_id,
                                        "连接结束"
                                    );
                                }
                                Err(error) => {
                                    record_proxy_error(&task_metrics, client_addr, &error);
                                }
                            }
                        });
                    }
                    Err(error) if is_temporary_accept_error(&error) => {
                        warn!(%error, "临时 accept 错误");
                    }
                    Err(error) => return Err(error.into()),
                }
            }
        }
    }

    drop(listener);
    semaphore.close();
    health_tasks.abort_all();
    while health_tasks.join_next().await.is_some() {}

    let grace = config.shutdown_grace();
    match timeout(grace, async {
        while let Some(task_result) = tasks.join_next().await {
            report_task_result(Some(task_result));
        }
    })
    .await
    {
        Ok(()) => info!("所有存量连接已结束"),
        Err(_) => {
            let remaining = tasks.len();
            warn!(remaining, ?grace, "优雅退出超时，取消剩余连接");
            tasks.abort_all();
            while tasks.join_next().await.is_some() {}
        }
    }

    log_snapshot(&metrics);
    Ok(())
}

async fn run_health_probe(target: crate::config::HealthProbeTarget, metrics: Arc<Metrics>) {
    let started = Instant::now();
    let result = match timeout(target.timeout, async {
        let mut stream = TcpStream::connect(&target.address).await?;
        if target.mode == HealthCheckMode::MinecraftStatus {
            probe_minecraft_status(
                &mut stream,
                &target.address,
                &target.minecraft_host,
                target.minecraft_protocol,
                target.proxy_protocol,
            )
            .await?;
        }
        Ok::<(), io::Error>(())
    })
    .await
    {
        Ok(Ok(())) => {
            metrics.health_check_succeeded();
            Ok(started.elapsed())
        }
        Ok(Err(error)) => {
            metrics.health_check_failed();
            Err(error)
        }
        Err(_) => {
            metrics.health_check_failed();
            Err(io::Error::new(io::ErrorKind::TimedOut, "健康检查超时"))
        }
    };
    target.complete(result);
}

fn record_proxy_error(metrics: &Metrics, client_addr: std::net::SocketAddr, error: &ProxyError) {
    if error.is_backend_failure() {
        metrics.backend_failed();
        warn!(%client_addr, %error, "后端连接失败");
    } else {
        metrics.forwarding_failed();
        debug!(%client_addr, %error, "连接转发异常结束");
    }
}

fn report_task_result(result: Option<Result<(), tokio::task::JoinError>>) {
    if let Some(Err(error)) = result {
        error!(%error, "连接任务异常退出");
    }
}

fn log_snapshot(metrics: &Metrics) {
    let snapshot = metrics.snapshot();
    info!(
        accepted = snapshot.accepted_connections,
        active = snapshot.active_connections,
        rejected = snapshot.rejected_connections,
        unmarked_handshakes = snapshot.unmarked_handshakes,
        legacy_forge_handshakes = snapshot.legacy_forge_handshakes,
        modern_forge_login_handshakes = snapshot.modern_forge_login_handshakes,
        configuration_forge_handshakes = snapshot.configuration_forge_handshakes,
        proxy_protocol_v1_headers = snapshot.proxy_protocol_v1_headers,
        proxy_protocol_v2_headers = snapshot.proxy_protocol_v2_headers,
        health_check_successes = snapshot.health_check_successes,
        health_check_failures = snapshot.health_check_failures,
        whitelist_denials = snapshot.whitelist_denials,
        local_status_responses = snapshot.local_status_responses,
        status_cache_hits = snapshot.status_cache_hits,
        status_fallbacks = snapshot.status_fallbacks,
        backend_attempt_failures = snapshot.backend_attempt_failures,
        backend_failovers = snapshot.backend_failovers,
        backend_failures = snapshot.backend_failures,
        forwarding_failures = snapshot.forwarding_failures,
        upload_bytes = snapshot.upload_bytes,
        download_bytes = snapshot.download_bytes,
        "代理统计"
    );
}

fn is_temporary_accept_error(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::ConnectionAborted
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::Interrupted
            | io::ErrorKind::WouldBlock
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AppConfig;

    #[tokio::test]
    async fn health_probe_records_success_and_failure() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let healthy_address = listener.local_addr().unwrap();
        let mut healthy_app = AppConfig::default();
        healthy_app.rules[0].backend = vec![healthy_address.to_string()];
        healthy_app.rules[0].health_check.enabled = true;
        healthy_app.rules[0].health_check.healthy_threshold = 1;
        let healthy = ForwardConfig::from_app(&healthy_app);
        let metrics = Arc::new(Metrics::default());
        let target = healthy.claim_due_health_probes(1).pop().unwrap();
        run_health_probe(target, Arc::clone(&metrics)).await;
        let snapshot = healthy.backend_health("default");
        assert_eq!(snapshot[0].health, crate::BackendHealthState::Healthy);
        assert_eq!(snapshot[0].health_check_successes, 1);

        let unavailable = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let unavailable_address = unavailable.local_addr().unwrap();
        drop(unavailable);
        let mut failed_app = AppConfig::default();
        failed_app.rules[0].backend = vec![unavailable_address.to_string()];
        failed_app.rules[0].health_check.enabled = true;
        failed_app.rules[0].health_check.unhealthy_threshold = 1;
        let failed = ForwardConfig::from_app(&failed_app);
        let target = failed.claim_due_health_probes(1).pop().unwrap();
        run_health_probe(target, Arc::clone(&metrics)).await;
        let snapshot = failed.backend_health("default");
        assert_eq!(snapshot[0].health, crate::BackendHealthState::Unhealthy);
        assert_eq!(snapshot[0].health_check_failures, 1);
        assert_eq!(metrics.snapshot().health_check_successes, 1);
        assert_eq!(metrics.snapshot().health_check_failures, 1);
    }
}
