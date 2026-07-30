use std::{
    io,
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use socket2::SockRef;
use thiserror::Error;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::TcpStream,
    time::{error::Elapsed, timeout},
};

use crate::{
    ForwardConfig, Metrics, ProxyProtocolVersion, RuleConfig, StatusConfig, StatusMode,
    StatusResponseConfig,
    config::{BackendConnectionGuard, BackendPoolState, CachedStatus, host_matches},
    metrics::HandshakeFlavor,
};

const MAX_HANDSHAKE_PACKET: usize = 8 * 1024;
const MAX_LOGIN_PACKET: usize = 64 * 1024;
const MAX_STATUS_PACKET: usize = 2 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
struct HandshakeInfo {
    protocol_version: usize,
    hostname: String,
    next_state: usize,
    flavor: HandshakeFlavor,
}

struct ManagedStatusRequest<'a> {
    client_handshake: &'a [u8],
    info: &'a HandshakeInfo,
    status: &'a StatusConfig,
    route: &'a RuleConfig,
    addresses: ConnectionAddresses,
}

struct StatusResolution<'a> {
    client_handshake: &'a [u8],
    request: &'a [u8],
    info: &'a HandshakeInfo,
    status: &'a StatusConfig,
    route: &'a RuleConfig,
    addresses: ConnectionAddresses,
}

struct BackendStatusFetch<'a> {
    handshake: &'a [u8],
    request: &'a [u8],
    backend_addr: &'a str,
    proxy_protocol: ProxyProtocolVersion,
    addresses: ConnectionAddresses,
    pool: &'a Arc<BackendPoolState>,
    index: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ConnectionAddresses {
    source: SocketAddr,
    destination: SocketAddr,
}

#[derive(Debug, Error)]
pub enum ProxyError {
    #[error("设置客户端 socket 失败: {0}")]
    ClientSocket(#[source] io::Error),
    #[error("读取 Minecraft 握手超时")]
    HandshakeTimeout(#[source] Elapsed),
    #[error("读取 Minecraft 握手失败: {0}")]
    Handshake(#[source] io::Error),
    #[error("握手域名未匹配任何已启用路由: {0}")]
    NoRoute(String),
    #[error("连接后端超时")]
    BackendTimeout(#[source] Elapsed),
    #[error("连接后端失败: {0}")]
    BackendConnect(#[source] io::Error),
    #[error("设置后端 socket 失败: {0}")]
    BackendSocket(#[source] io::Error),
    #[error("向后端发送 PROXY Protocol 头失败: {0}")]
    BackendProxyProtocol(#[source] io::Error),
    #[error("后端状态响应无效: {0}")]
    BackendStatus(#[source] io::Error),
    #[error("双向转发失败: {0}")]
    Forward(#[source] io::Error),
}

impl ProxyError {
    pub fn is_backend_failure(&self) -> bool {
        matches!(
            self,
            Self::BackendTimeout(_)
                | Self::BackendConnect(_)
                | Self::BackendSocket(_)
                | Self::BackendProxyProtocol(_)
                | Self::BackendStatus(_)
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransferReport {
    pub upload_bytes: u64,
    pub download_bytes: u64,
    pub elapsed_millis: u128,
    pub backend: String,
    pub route_id: String,
}

pub async fn proxy_connection(
    mut client: TcpStream,
    config: &ForwardConfig,
    metrics: Arc<Metrics>,
) -> Result<TransferReport, ProxyError> {
    configure_stream(&client, config).map_err(ProxyError::ClientSocket)?;
    let addresses = ConnectionAddresses {
        source: client.peer_addr().map_err(ProxyError::ClientSocket)?,
        destination: client.local_addr().map_err(ProxyError::ClientSocket)?,
    };

    let handshake = timeout(config.handshake_timeout(), read_handshake(&mut client))
        .await
        .map_err(ProxyError::HandshakeTimeout)?
        .map_err(ProxyError::Handshake)?;
    let handshake_info = parse_handshake(&handshake);
    if let Some(info) = &handshake_info {
        metrics.observed_handshake(info.flavor);
    }
    let hostname = handshake_info.as_ref().map(|info| info.hostname.as_str());
    let route = select_route(&config.routes, hostname)
        .ok_or_else(|| ProxyError::NoRoute(hostname.unwrap_or("<legacy-ping>").to_string()))?;
    let route_id = route.id.clone();

    if let (Some(info), Some(status)) = (&handshake_info, &route.status) {
        if info.next_state == 1 {
            return serve_managed_status(
                client,
                config,
                ManagedStatusRequest {
                    client_handshake: &handshake,
                    info,
                    status,
                    route,
                    addresses,
                },
                metrics,
            )
            .await;
        }
    }

    let handshake_len = handshake.len();
    let mut prefetched = handshake;
    if let Some(info) = &handshake_info {
        if info.next_state == 2 && route.whitelist_enabled {
            let login_start = timeout(
                config.handshake_timeout(),
                read_packet(&mut client, MAX_LOGIN_PACKET),
            )
            .await
            .map_err(ProxyError::HandshakeTimeout)?
            .map_err(ProxyError::Handshake)?;
            let player = parse_login_username(&login_start).ok_or_else(|| {
                ProxyError::Handshake(invalid_data("无法解析 Minecraft Login Start 玩家名"))
            })?;
            if !route
                .whitelist
                .iter()
                .any(|allowed| allowed.eq_ignore_ascii_case(&player))
            {
                let response = login_disconnect_packet(&route.whitelist_message)
                    .map_err(ProxyError::Handshake)?;
                client
                    .write_all(&response)
                    .await
                    .map_err(ProxyError::Forward)?;
                metrics.uploaded((prefetched.len() + login_start.len()) as u64);
                metrics.downloaded(response.len() as u64);
                metrics.whitelist_denied();
                return Ok(TransferReport {
                    upload_bytes: (prefetched.len() + login_start.len()) as u64,
                    download_bytes: response.len() as u64,
                    elapsed_millis: 0,
                    backend: "<whitelist-denied>".to_string(),
                    route_id,
                });
            }
            prefetched.extend_from_slice(&login_start);
        }
    }

    let (mut backend, backend_addr, _backend_guard) =
        connect_route_backend(config, route, addresses, &metrics).await?;
    if route.modify_virtual_host {
        let mut rewritten = rewrite_handshake_hostname(
            &prefetched[..handshake_len],
            backend_handshake_host(&backend_addr),
        )
        .map_err(ProxyError::Handshake)?;
        rewritten.extend_from_slice(&prefetched[handshake_len..]);
        prefetched = rewritten;
    }
    if !prefetched.is_empty() {
        backend
            .write_all(&prefetched)
            .await
            .map_err(ProxyError::Forward)?;
        metrics.uploaded(prefetched.len() as u64);
    }

    let started = Instant::now();
    let (client_read, client_write) = client.into_split();
    let (backend_read, backend_write) = backend.into_split();
    let upload_metrics = Arc::clone(&metrics);
    let download_metrics = Arc::clone(&metrics);

    let upload = copy_direction(
        client_read,
        backend_write,
        config.copy_buffer_bytes,
        move |bytes| upload_metrics.uploaded(bytes),
    );
    let download = copy_direction(
        backend_read,
        client_write,
        config.copy_buffer_bytes,
        move |bytes| download_metrics.downloaded(bytes),
    );

    let (mut upload_bytes, download_bytes) =
        tokio::try_join!(upload, download).map_err(ProxyError::Forward)?;
    upload_bytes += prefetched.len() as u64;

    Ok(TransferReport {
        upload_bytes,
        download_bytes,
        elapsed_millis: started.elapsed().as_millis(),
        backend: backend_addr,
        route_id,
    })
}

async fn connect_route_backend(
    config: &ForwardConfig,
    route: &RuleConfig,
    addresses: ConnectionAddresses,
    metrics: &Metrics,
) -> Result<(TcpStream, String, BackendConnectionGuard), ProxyError> {
    let pool = config
        .backend_pools
        .get(&route.id)
        .expect("每条运行路由都必须有后端状态池");
    let mut last_error = None;
    let mut failed_attempts = 0_u64;
    for index in pool.candidate_indices(route.strategy) {
        let backend_addr = pool.address(index);
        match connect_backend(config, backend_addr).await {
            Ok((mut backend, latency)) => {
                if let Err(error) =
                    write_proxy_protocol_header(&mut backend, route.proxy_protocol, addresses).await
                {
                    pool.failed(index);
                    metrics.backend_attempt_failed();
                    failed_attempts += 1;
                    last_error = Some(ProxyError::BackendProxyProtocol(error));
                    continue;
                }
                metrics.proxy_protocol_header(route.proxy_protocol);
                if failed_attempts > 0 {
                    metrics.backend_failover();
                }
                return Ok((
                    backend,
                    backend_addr.to_string(),
                    pool.connected(index, latency),
                ));
            }
            Err(error) => {
                pool.failed(index);
                metrics.backend_attempt_failed();
                failed_attempts += 1;
                last_error = Some(error);
            }
        }
    }
    Err(last_error.expect("已校验的路由至少包含一个后端"))
}

async fn connect_backend(
    config: &ForwardConfig,
    backend_addr: &str,
) -> Result<(TcpStream, Duration), ProxyError> {
    let started = Instant::now();
    let backend = timeout(config.connect_timeout(), TcpStream::connect(backend_addr))
        .await
        .map_err(ProxyError::BackendTimeout)?
        .map_err(ProxyError::BackendConnect)?;
    configure_stream(&backend, config).map_err(ProxyError::BackendSocket)?;
    Ok((backend, started.elapsed()))
}

async fn write_proxy_protocol_header(
    backend: &mut TcpStream,
    version: ProxyProtocolVersion,
    addresses: ConnectionAddresses,
) -> io::Result<()> {
    let header = match version {
        ProxyProtocolVersion::Off => return Ok(()),
        ProxyProtocolVersion::V1 => proxy_protocol_v1_header(addresses),
        ProxyProtocolVersion::V2 => proxy_protocol_v2_header(addresses),
    };
    backend.write_all(&header).await
}

fn proxy_protocol_v1_header(addresses: ConnectionAddresses) -> Vec<u8> {
    match (addresses.source.ip(), addresses.destination.ip()) {
        (IpAddr::V4(source), IpAddr::V4(destination)) => format!(
            "PROXY TCP4 {source} {destination} {} {}\r\n",
            addresses.source.port(),
            addresses.destination.port()
        )
        .into_bytes(),
        (IpAddr::V6(source), IpAddr::V6(destination)) => format!(
            "PROXY TCP6 {source} {destination} {} {}\r\n",
            addresses.source.port(),
            addresses.destination.port()
        )
        .into_bytes(),
        _ => b"PROXY UNKNOWN\r\n".to_vec(),
    }
}

fn proxy_protocol_v2_header(addresses: ConnectionAddresses) -> Vec<u8> {
    const SIGNATURE: &[u8; 12] = b"\r\n\r\n\0\r\nQUIT\n";
    let mut header = Vec::with_capacity(52);
    header.extend_from_slice(SIGNATURE);
    header.push(0x21);
    match (addresses.source.ip(), addresses.destination.ip()) {
        (IpAddr::V4(source), IpAddr::V4(destination)) => {
            header.push(0x11);
            header.extend_from_slice(&12_u16.to_be_bytes());
            header.extend_from_slice(&source.octets());
            header.extend_from_slice(&destination.octets());
        }
        (IpAddr::V6(source), IpAddr::V6(destination)) => {
            header.push(0x21);
            header.extend_from_slice(&36_u16.to_be_bytes());
            header.extend_from_slice(&source.octets());
            header.extend_from_slice(&destination.octets());
        }
        _ => {
            header.push(0x00);
            header.extend_from_slice(&0_u16.to_be_bytes());
            return header;
        }
    }
    header.extend_from_slice(&addresses.source.port().to_be_bytes());
    header.extend_from_slice(&addresses.destination.port().to_be_bytes());
    header
}

async fn read_handshake(client: &mut TcpStream) -> io::Result<Vec<u8>> {
    let first = client.read_u8().await?;
    if first == 0xfe {
        return Ok(vec![first]);
    }
    read_packet_after_first(client, first, MAX_HANDSHAKE_PACKET).await
}

async fn read_packet(client: &mut TcpStream, max_length: usize) -> io::Result<Vec<u8>> {
    let first = client.read_u8().await?;
    read_packet_after_first(client, first, max_length).await
}

async fn read_packet_after_first(
    client: &mut TcpStream,
    first: u8,
    max_length: usize,
) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(512);
    bytes.push(first);

    let mut packet_length = usize::from(first & 0x7f);
    let mut shift = 7;
    let mut current = first;
    while current & 0x80 != 0 {
        if shift >= 35 {
            return Err(invalid_data("握手包长度 VarInt 过长"));
        }
        current = client.read_u8().await?;
        bytes.push(current);
        packet_length |= usize::from(current & 0x7f) << shift;
        shift += 7;
    }
    if packet_length == 0 || packet_length > max_length {
        return Err(invalid_data("Minecraft 包长度超出限制"));
    }

    let header_length = bytes.len();
    bytes.resize(header_length + packet_length, 0);
    client.read_exact(&mut bytes[header_length..]).await?;
    Ok(bytes)
}

fn parse_handshake(packet: &[u8]) -> Option<HandshakeInfo> {
    if packet.first().copied() == Some(0xfe) {
        return None;
    }
    let mut cursor = 0;
    let packet_length = read_varint(packet, &mut cursor)?;
    if packet_length == 0 || packet_length > MAX_HANDSHAKE_PACKET {
        return None;
    }
    let packet_end = cursor.checked_add(packet_length)?;
    if packet_end > packet.len() || read_varint(packet, &mut cursor)? != 0 {
        return None;
    }
    let protocol_version = read_varint(packet, &mut cursor)?;
    let hostname_length = read_varint(packet, &mut cursor)?;
    if hostname_length == 0 || hostname_length > 1024 {
        return None;
    }
    let hostname_end = cursor.checked_add(hostname_length)?;
    if hostname_end.checked_add(2)? > packet_end {
        return None;
    }
    let raw = std::str::from_utf8(&packet[cursor..hostname_end]).ok()?;
    let hostname = raw
        .split('\0')
        .next()?
        .trim()
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if hostname.is_empty() {
        return None;
    }
    let flavor = classify_handshake_flavor(raw);
    cursor = hostname_end + 2;
    let next_state = read_varint(packet, &mut cursor)?;
    (cursor == packet_end).then_some(HandshakeInfo {
        protocol_version,
        hostname,
        next_state,
        flavor,
    })
}

fn classify_handshake_flavor(raw_hostname: &str) -> HandshakeFlavor {
    let marker_parts = raw_hostname.split('\0').skip(1);
    let mut flavor = HandshakeFlavor::Unmarked;
    for marker in marker_parts {
        match marker {
            "FML" => return HandshakeFlavor::LegacyForge,
            "FML2" | "FML3" => flavor = HandshakeFlavor::ModernForgeLogin,
            marker if marker.starts_with("FORGE") => {
                return HandshakeFlavor::ConfigurationForge;
            }
            _ => {}
        }
    }
    flavor
}

fn read_varint(bytes: &[u8], cursor: &mut usize) -> Option<usize> {
    let mut value = 0_usize;
    for shift in (0..35).step_by(7) {
        let byte = *bytes.get(*cursor)?;
        *cursor += 1;
        value |= usize::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some(value);
        }
    }
    None
}

fn rewrite_handshake_hostname(packet: &[u8], new_hostname: &str) -> io::Result<Vec<u8>> {
    if packet.first().copied() == Some(0xfe) {
        return Err(invalid_data("旧版 Ping 不支持改写握手 Host"));
    }

    let mut cursor = 0;
    let packet_length =
        read_varint(packet, &mut cursor).ok_or_else(|| invalid_data("无法读取握手包长度"))?;
    let payload_start = cursor;
    let packet_end = payload_start
        .checked_add(packet_length)
        .ok_or_else(|| invalid_data("握手包长度溢出"))?;
    if packet_end > packet.len() || read_varint(packet, &mut cursor) != Some(0) {
        return Err(invalid_data("不是有效的 Minecraft Handshake"));
    }
    read_varint(packet, &mut cursor).ok_or_else(|| invalid_data("无法读取协议版本"))?;
    let hostname_length_field = cursor;
    let hostname_length =
        read_varint(packet, &mut cursor).ok_or_else(|| invalid_data("无法读取握手 Host 长度"))?;
    let hostname_start = cursor;
    let hostname_end = hostname_start
        .checked_add(hostname_length)
        .ok_or_else(|| invalid_data("握手 Host 长度溢出"))?;
    if hostname_end > packet_end {
        return Err(invalid_data("握手 Host 超出包边界"));
    }

    let old_hostname = &packet[hostname_start..hostname_end];
    let suffix = old_hostname
        .iter()
        .position(|byte| *byte == 0)
        .map_or(&[][..], |index| &old_hostname[index..]);
    let new_hostname = new_hostname.as_bytes();
    let new_length = new_hostname
        .len()
        .checked_add(suffix.len())
        .ok_or_else(|| invalid_data("改写后的握手 Host 过长"))?;
    if new_length > 1024 {
        return Err(invalid_data("改写后的握手 Host 超过 1024 字节"));
    }

    let mut payload = Vec::with_capacity(packet_length + new_length);
    payload.extend_from_slice(&packet[payload_start..hostname_length_field]);
    write_varint(new_length, &mut payload);
    payload.extend_from_slice(new_hostname);
    payload.extend_from_slice(suffix);
    payload.extend_from_slice(&packet[hostname_end..packet_end]);

    let mut rewritten = Vec::with_capacity(payload.len() + 5);
    write_varint(payload.len(), &mut rewritten);
    rewritten.extend_from_slice(&payload);
    Ok(rewritten)
}

fn write_varint(mut value: usize, output: &mut Vec<u8>) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        output.push(byte);
        if value == 0 {
            break;
        }
    }
}

fn write_signed_varint(value: i32, output: &mut Vec<u8>) {
    let mut value = value as u32;
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        output.push(byte);
        if value == 0 {
            break;
        }
    }
}

async fn serve_managed_status(
    mut client: TcpStream,
    config: &ForwardConfig,
    managed: ManagedStatusRequest<'_>,
    metrics: Arc<Metrics>,
) -> Result<TransferReport, ProxyError> {
    let started = Instant::now();
    let request = timeout(
        config.handshake_timeout(),
        read_packet(&mut client, MAX_HANDSHAKE_PACKET),
    )
    .await
    .map_err(ProxyError::HandshakeTimeout)?
    .map_err(ProxyError::Handshake)?;
    if packet_id(&request) != Some(0) {
        return Err(ProxyError::Handshake(invalid_data(
            "状态阶段首包不是 Status Request",
        )));
    }

    let response_json = resolve_status_response(
        config,
        StatusResolution {
            client_handshake: managed.client_handshake,
            request: &request,
            info: managed.info,
            status: managed.status,
            route: managed.route,
            addresses: managed.addresses,
        },
        &metrics,
    )
    .await?
    .to_string();
    let response = string_packet(0, &response_json).map_err(ProxyError::Handshake)?;
    client
        .write_all(&response)
        .await
        .map_err(ProxyError::Forward)?;

    let mut upload_bytes = managed.client_handshake.len() + request.len();
    let mut download_bytes = response.len();
    if let Ok(Ok(ping)) = timeout(
        config.handshake_timeout(),
        read_packet(&mut client, MAX_HANDSHAKE_PACKET),
    )
    .await
    {
        if packet_id(&ping) == Some(1) {
            client.write_all(&ping).await.map_err(ProxyError::Forward)?;
            upload_bytes += ping.len();
            download_bytes += ping.len();
        }
    }

    metrics.uploaded(upload_bytes as u64);
    metrics.downloaded(download_bytes as u64);
    metrics.local_status_responded();
    Ok(TransferReport {
        upload_bytes: upload_bytes as u64,
        download_bytes: download_bytes as u64,
        elapsed_millis: started.elapsed().as_millis(),
        backend: "<local-status>".to_string(),
        route_id: managed.route.id.clone(),
    })
}

async fn resolve_status_response(
    config: &ForwardConfig,
    resolution: StatusResolution<'_>,
    metrics: &Metrics,
) -> Result<Value, ProxyError> {
    let client_protocol = i32::try_from(resolution.info.protocol_version).unwrap_or(i32::MAX);
    match resolution.status.mode {
        StatusMode::Custom => {
            let mut response = default_custom_status(client_protocol);
            apply_status_fields(
                &mut response,
                resolution.status.motd.as_deref(),
                resolution.status.version_name.as_deref(),
                resolution.status.protocol,
                resolution.status.online,
                resolution.status.max,
            )
            .map_err(ProxyError::Handshake)?;
            Ok(response)
        }
        StatusMode::Backend => {
            let backend_result = cached_or_fetch_route_status(config, &resolution, metrics).await;
            let mut response = match backend_result {
                Ok(response) => response,
                Err(error) => {
                    let Some(fallback) = &resolution.status.fallback else {
                        return Err(error);
                    };
                    metrics.backend_failed();
                    metrics.status_fallback();
                    let mut fallback_response = default_fallback_status(client_protocol);
                    apply_response_config(&mut fallback_response, fallback)
                        .map_err(ProxyError::Handshake)?;
                    return Ok(fallback_response);
                }
            };
            apply_status_fields(
                &mut response,
                resolution.status.motd.as_deref(),
                resolution.status.version_name.as_deref(),
                resolution.status.protocol,
                resolution.status.online,
                resolution.status.max,
            )
            .map_err(ProxyError::Handshake)?;
            Ok(response)
        }
    }
}

async fn cached_or_fetch_route_status(
    config: &ForwardConfig,
    resolution: &StatusResolution<'_>,
    metrics: &Metrics,
) -> Result<Value, ProxyError> {
    let pool = config
        .backend_pools
        .get(&resolution.route.id)
        .expect("每条运行路由都必须有后端状态池");
    let mut last_error = None;
    let mut failed_attempts = 0_u64;

    for index in pool.candidate_indices(resolution.route.strategy) {
        let backend_addr = pool.address(index);
        let cache_key = format!("{backend_addr}\0{}", resolution.info.protocol_version);
        if resolution.status.cache_ttl_secs > 0 {
            let mut cache = config.status_cache.lock().await;
            if let Some(cached) = cache.get(&cache_key) {
                if cached.expires_at > Instant::now() {
                    match serde_json::from_str(&cached.response_json) {
                        Ok(response) => {
                            metrics.status_cache_hit();
                            if failed_attempts > 0 {
                                metrics.backend_failover();
                            }
                            return Ok(response);
                        }
                        Err(_) => {
                            cache.remove(&cache_key);
                        }
                    }
                } else {
                    cache.remove(&cache_key);
                }
            }
        }

        let handshake = if resolution.route.modify_virtual_host {
            rewrite_handshake_hostname(
                resolution.client_handshake,
                backend_handshake_host(backend_addr),
            )
            .map_err(ProxyError::Handshake)?
        } else {
            resolution.client_handshake.to_vec()
        };
        let result = fetch_backend_status(
            config,
            BackendStatusFetch {
                handshake: &handshake,
                request: resolution.request,
                backend_addr,
                proxy_protocol: resolution.route.proxy_protocol,
                addresses: resolution.addresses,
                pool,
                index,
            },
            metrics,
        )
        .await;
        match result {
            Ok(response_json) => {
                let response = serde_json::from_str::<Value>(&response_json)
                    .map_err(|_| invalid_data("后端 Status JSON 无效"))
                    .and_then(|response| {
                        response
                            .is_object()
                            .then_some(response)
                            .ok_or_else(|| invalid_data("后端 Status JSON 顶层不是对象"))
                    });
                match response {
                    Ok(response) => {
                        if resolution.status.cache_ttl_secs > 0 {
                            let expires_at = Instant::now()
                                + Duration::from_secs(resolution.status.cache_ttl_secs as u64);
                            config.status_cache.lock().await.insert(
                                cache_key,
                                CachedStatus {
                                    expires_at,
                                    response_json,
                                },
                            );
                        }
                        if failed_attempts > 0 {
                            metrics.backend_failover();
                        }
                        return Ok(response);
                    }
                    Err(error) => {
                        let error = ProxyError::BackendStatus(error);
                        pool.failed(index);
                        metrics.backend_attempt_failed();
                        failed_attempts += 1;
                        last_error = Some(error);
                    }
                }
            }
            Err(error) => {
                pool.failed(index);
                metrics.backend_attempt_failed();
                failed_attempts += 1;
                last_error = Some(error);
            }
        }
    }
    Err(last_error.expect("已校验的路由至少包含一个后端"))
}

async fn fetch_backend_status(
    config: &ForwardConfig,
    fetch: BackendStatusFetch<'_>,
    metrics: &Metrics,
) -> Result<String, ProxyError> {
    let (mut backend, latency) = connect_backend(config, fetch.backend_addr).await?;
    write_proxy_protocol_header(&mut backend, fetch.proxy_protocol, fetch.addresses)
        .await
        .map_err(ProxyError::BackendProxyProtocol)?;
    metrics.proxy_protocol_header(fetch.proxy_protocol);
    let _guard = fetch.pool.connected(fetch.index, latency);
    backend
        .write_all(fetch.handshake)
        .await
        .map_err(ProxyError::BackendStatus)?;
    backend
        .write_all(fetch.request)
        .await
        .map_err(ProxyError::BackendStatus)?;
    let response = timeout(
        config.handshake_timeout(),
        read_packet(&mut backend, MAX_STATUS_PACKET),
    )
    .await
    .map_err(ProxyError::BackendTimeout)?
    .map_err(ProxyError::BackendStatus)?;
    packet_string(&response, 0)
        .map(str::to_string)
        .ok_or_else(|| ProxyError::BackendStatus(invalid_data("无法解析后端 Status Response")))
}

pub(crate) async fn probe_minecraft_status(
    backend: &mut TcpStream,
    backend_addr: &str,
    hostname: &str,
    protocol: i32,
    proxy_protocol: ProxyProtocolVersion,
) -> io::Result<()> {
    if hostname.is_empty() || hostname.len() > 255 {
        return Err(invalid_data("Minecraft 健康检查 Host 长度无效"));
    }
    let source = backend.local_addr()?;
    let destination = backend.peer_addr()?;
    write_proxy_protocol_header(
        backend,
        proxy_protocol,
        ConnectionAddresses {
            source,
            destination,
        },
    )
    .await?;

    let port = backend_addr
        .rsplit_once(':')
        .and_then(|(_, port)| port.parse::<u16>().ok())
        .ok_or_else(|| invalid_data("Minecraft 健康检查后端端口无效"))?;
    let mut handshake_payload = Vec::with_capacity(hostname.len() + 16);
    write_varint(0, &mut handshake_payload);
    write_signed_varint(protocol, &mut handshake_payload);
    write_varint(hostname.len(), &mut handshake_payload);
    handshake_payload.extend_from_slice(hostname.as_bytes());
    handshake_payload.extend_from_slice(&port.to_be_bytes());
    write_varint(1, &mut handshake_payload);
    let mut handshake = Vec::with_capacity(handshake_payload.len() + 5);
    write_varint(handshake_payload.len(), &mut handshake);
    handshake.extend_from_slice(&handshake_payload);

    backend.write_all(&handshake).await?;
    backend.write_all(&[0x01, 0x00]).await?;
    let response = read_packet(backend, MAX_STATUS_PACKET).await?;
    let response_json = packet_string(&response, 0)
        .ok_or_else(|| invalid_data("Minecraft 健康检查 Status Response 无效"))?;
    let response: Value = serde_json::from_str(response_json)
        .map_err(|_| invalid_data("Minecraft 健康检查 Status JSON 无效"))?;
    let root = response
        .as_object()
        .ok_or_else(|| invalid_data("Minecraft 健康检查 Status JSON 顶层不是对象"))?;
    if !root.get("version").is_some_and(Value::is_object)
        || !root.get("players").is_some_and(Value::is_object)
        || !root.contains_key("description")
    {
        return Err(invalid_data(
            "Minecraft 健康检查 Status JSON 缺少 version、players 或 description",
        ));
    }

    const PING_VALUE: i64 = 0x4d43_5052_4f42_4501;
    let mut ping_payload = Vec::with_capacity(9);
    write_varint(1, &mut ping_payload);
    ping_payload.extend_from_slice(&PING_VALUE.to_be_bytes());
    let mut ping = Vec::with_capacity(10);
    write_varint(ping_payload.len(), &mut ping);
    ping.extend_from_slice(&ping_payload);
    backend.write_all(&ping).await?;
    let pong = read_packet(backend, MAX_HANDSHAKE_PACKET).await?;
    let mut cursor = 0;
    let packet_length =
        read_varint(&pong, &mut cursor).ok_or_else(|| invalid_data("Pong 长度无效"))?;
    if cursor.checked_add(packet_length) != Some(pong.len())
        || read_varint(&pong, &mut cursor) != Some(1)
        || pong.get(cursor..) != Some(PING_VALUE.to_be_bytes().as_slice())
    {
        return Err(invalid_data("Minecraft 健康检查 Pong 不匹配"));
    }
    Ok(())
}

fn packet_string(packet: &[u8], expected_packet_id: usize) -> Option<&str> {
    let mut cursor = 0;
    let packet_length = read_varint(packet, &mut cursor)?;
    let packet_end = cursor.checked_add(packet_length)?;
    if packet_end != packet.len() || read_varint(packet, &mut cursor)? != expected_packet_id {
        return None;
    }
    let string_length = read_varint(packet, &mut cursor)?;
    let string_end = cursor.checked_add(string_length)?;
    if string_end != packet_end {
        return None;
    }
    std::str::from_utf8(&packet[cursor..string_end]).ok()
}

fn default_custom_status(protocol: i32) -> Value {
    json!({
        "version": { "name": "MC Relay", "protocol": protocol },
        "players": { "max": 100, "online": 0, "sample": [] },
        "description": { "text": "§aMinecraft Server" },
    })
}

fn default_fallback_status(protocol: i32) -> Value {
    json!({
        "version": { "name": "§cBackend offline", "protocol": protocol },
        "players": { "max": 0, "online": 0, "sample": [] },
        "description": { "text": "§c后端服务器暂时离线，请稍后重试。" },
    })
}

fn apply_response_config(response: &mut Value, config: &StatusResponseConfig) -> io::Result<()> {
    apply_status_fields(
        response,
        config.motd.as_deref(),
        config.version_name.as_deref(),
        config.protocol,
        config.online,
        config.max,
    )
}

fn apply_status_fields(
    response: &mut Value,
    motd: Option<&str>,
    version_name: Option<&str>,
    protocol: Option<i32>,
    online: Option<u32>,
    max: Option<u32>,
) -> io::Result<()> {
    let root = response
        .as_object_mut()
        .ok_or_else(|| invalid_data("Status JSON 顶层不是对象"))?;
    if let Some(motd) = motd {
        root.insert("description".to_string(), parse_text_component(motd));
    }
    if version_name.is_some() || protocol.is_some() {
        let version = root
            .entry("version")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .ok_or_else(|| invalid_data("Status JSON version 不是对象"))?;
        if let Some(version_name) = version_name {
            version.insert("name".to_string(), json!(version_name));
        }
        if let Some(protocol) = protocol {
            version.insert("protocol".to_string(), json!(protocol));
        }
    }
    if online.is_some() || max.is_some() {
        let players = root
            .entry("players")
            .or_insert_with(|| json!({ "sample": [] }))
            .as_object_mut()
            .ok_or_else(|| invalid_data("Status JSON players 不是对象"))?;
        if let Some(online) = online {
            players.insert("online".to_string(), json!(online));
        }
        if let Some(max) = max {
            players.insert("max".to_string(), json!(max));
        }
    }
    Ok(())
}

fn packet_id(packet: &[u8]) -> Option<usize> {
    let mut cursor = 0;
    let packet_length = read_varint(packet, &mut cursor)?;
    let packet_end = cursor.checked_add(packet_length)?;
    if packet_end != packet.len() {
        return None;
    }
    read_varint(packet, &mut cursor)
}

fn parse_login_username(packet: &[u8]) -> Option<String> {
    let mut cursor = 0;
    let packet_length = read_varint(packet, &mut cursor)?;
    let packet_end = cursor.checked_add(packet_length)?;
    if packet_end != packet.len() || read_varint(packet, &mut cursor)? != 0 {
        return None;
    }
    let username_length = read_varint(packet, &mut cursor)?;
    if username_length == 0 || username_length > 16 {
        return None;
    }
    let username_end = cursor.checked_add(username_length)?;
    if username_end > packet_end {
        return None;
    }
    std::str::from_utf8(&packet[cursor..username_end])
        .ok()
        .map(str::to_string)
}

fn login_disconnect_packet(message: &str) -> io::Result<Vec<u8>> {
    string_packet(0, &parse_text_component(message).to_string())
}

fn parse_text_component(message: &str) -> Value {
    serde_json::from_str(message).unwrap_or_else(|_| json!({ "text": message }))
}

fn string_packet(packet_id: usize, value: &str) -> io::Result<Vec<u8>> {
    if value.len() > 32_767 {
        return Err(invalid_data("Minecraft 字符串超过 32767 字节"));
    }
    let mut payload = Vec::with_capacity(value.len() + 10);
    write_varint(packet_id, &mut payload);
    write_varint(value.len(), &mut payload);
    payload.extend_from_slice(value.as_bytes());
    let mut packet = Vec::with_capacity(payload.len() + 5);
    write_varint(payload.len(), &mut packet);
    packet.extend_from_slice(&payload);
    Ok(packet)
}

fn backend_handshake_host(backend: &str) -> &str {
    let host = backend.rsplit_once(':').map_or(backend, |(host, _)| host);
    host.strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(host)
}

fn select_route<'a>(routes: &'a [RuleConfig], hostname: Option<&str>) -> Option<&'a RuleConfig> {
    let hostname = hostname?;
    routes.iter().find(|route| {
        route
            .host
            .iter()
            .any(|pattern| host_matches(pattern, hostname))
    })
}

fn invalid_data(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

async fn copy_direction<R, W, F>(
    mut reader: R,
    mut writer: W,
    buffer_size: usize,
    mut record: F,
) -> io::Result<u64>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
    F: FnMut(u64),
{
    let mut buffer = vec![0_u8; buffer_size];
    let mut total = 0_u64;
    loop {
        let count = reader.read(&mut buffer).await?;
        if count == 0 {
            writer.shutdown().await?;
            return Ok(total);
        }
        writer.write_all(&buffer[..count]).await?;
        let count = count as u64;
        total += count;
        record(count);
    }
}

fn configure_stream(stream: &TcpStream, config: &ForwardConfig) -> io::Result<()> {
    stream.set_nodelay(config.tcp_nodelay)?;
    if config.socket_buffer_bytes > 0 {
        let socket = SockRef::from(stream);
        socket.set_recv_buffer_size(config.socket_buffer_bytes)?;
        socket.set_send_buffer_size(config.socket_buffer_bytes)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    fn handshake(hostname: &str) -> Vec<u8> {
        let mut payload = vec![0, 0xf9, 0x05, hostname.len() as u8];
        payload.extend_from_slice(hostname.as_bytes());
        payload.extend_from_slice(&[0x63, 0xdd, 1]);
        let mut packet = vec![payload.len() as u8];
        packet.extend(payload);
        packet
    }

    #[test]
    fn extracts_hostname_from_handshake() {
        assert_eq!(
            parse_handshake(&handshake("Play.Example.COM"))
                .map(|info| info.hostname)
                .as_deref(),
            Some("play.example.com")
        );
    }

    #[test]
    fn classifies_gate_compatible_forge_markers() {
        let cases = [
            ("play.example.com", HandshakeFlavor::Unmarked),
            ("play.example.com\0FML\0", HandshakeFlavor::LegacyForge),
            (
                "play.example.com\0FML2\0",
                HandshakeFlavor::ModernForgeLogin,
            ),
            (
                "play.example.com\0FML3\0",
                HandshakeFlavor::ModernForgeLogin,
            ),
            (
                "play.example.com\0FORGE",
                HandshakeFlavor::ConfigurationForge,
            ),
            (
                "play.example.com\0FORGE2",
                HandshakeFlavor::ConfigurationForge,
            ),
        ];
        for (hostname, expected) in cases {
            let parsed = parse_handshake(&handshake(hostname)).unwrap();
            assert_eq!(parsed.hostname, "play.example.com");
            assert_eq!(parsed.flavor, expected, "hostname: {hostname:?}");
        }
        assert_eq!(
            classify_handshake_flavor("forge.example.com"),
            HandshakeFlavor::Unmarked
        );
    }

    #[test]
    fn rewrites_handshake_hostname_and_preserves_packet() {
        for marker in ["", "\0FML\0", "\0FML2\0", "\0FML3\0", "\0FORGE", "\0FORGE2"] {
            let packet = handshake(&format!("hyp.mc.lic6.top{marker}"));
            let rewritten = rewrite_handshake_hostname(&packet, "mc.hypixel.net").unwrap();
            let expected_host = format!("mc.hypixel.net{marker}");
            let expected = handshake(&expected_host);
            assert_eq!(rewritten, expected, "marker: {marker:?}");
            assert_eq!(
                parse_handshake(&rewritten)
                    .map(|info| info.hostname)
                    .as_deref(),
                Some("mc.hypixel.net")
            );
        }
        assert_eq!(
            backend_handshake_host("backend.example.com:25565"),
            "backend.example.com"
        );
    }

    #[test]
    fn encodes_proxy_protocol_v1_for_ipv4_and_ipv6() {
        let ipv4 = ConnectionAddresses {
            source: "192.0.2.10:54321".parse().unwrap(),
            destination: "198.51.100.20:25565".parse().unwrap(),
        };
        assert_eq!(
            proxy_protocol_v1_header(ipv4),
            b"PROXY TCP4 192.0.2.10 198.51.100.20 54321 25565\r\n"
        );

        let ipv6 = ConnectionAddresses {
            source: "[2001:db8::10]:443".parse().unwrap(),
            destination: "[2001:db8::20]:25565".parse().unwrap(),
        };
        assert_eq!(
            proxy_protocol_v1_header(ipv6),
            b"PROXY TCP6 2001:db8::10 2001:db8::20 443 25565\r\n"
        );
    }

    #[test]
    fn encodes_proxy_protocol_v2_for_ipv4_and_ipv6() {
        let ipv4 = ConnectionAddresses {
            source: "192.0.2.10:54321".parse().unwrap(),
            destination: "198.51.100.20:25565".parse().unwrap(),
        };
        let mut expected_ipv4 = b"\r\n\r\n\0\r\nQUIT\n\x21\x11\0\x0c".to_vec();
        expected_ipv4.extend_from_slice(&[192, 0, 2, 10, 198, 51, 100, 20]);
        expected_ipv4.extend_from_slice(&54321_u16.to_be_bytes());
        expected_ipv4.extend_from_slice(&25565_u16.to_be_bytes());
        assert_eq!(proxy_protocol_v2_header(ipv4), expected_ipv4);

        let ipv6 = ConnectionAddresses {
            source: "[2001:db8::10]:443".parse().unwrap(),
            destination: "[2001:db8::20]:25565".parse().unwrap(),
        };
        let mut expected_ipv6 = b"\r\n\r\n\0\r\nQUIT\n\x21\x21\0\x24".to_vec();
        expected_ipv6.extend_from_slice(
            &"2001:db8::10"
                .parse::<std::net::Ipv6Addr>()
                .unwrap()
                .octets(),
        );
        expected_ipv6.extend_from_slice(
            &"2001:db8::20"
                .parse::<std::net::Ipv6Addr>()
                .unwrap()
                .octets(),
        );
        expected_ipv6.extend_from_slice(&443_u16.to_be_bytes());
        expected_ipv6.extend_from_slice(&25565_u16.to_be_bytes());
        assert_eq!(proxy_protocol_v2_header(ipv6), expected_ipv6);
    }

    #[test]
    fn proxy_protocol_uses_unknown_family_for_mixed_addresses() {
        let mixed = ConnectionAddresses {
            source: "192.0.2.10:54321".parse().unwrap(),
            destination: "[2001:db8::20]:25565".parse().unwrap(),
        };
        assert_eq!(proxy_protocol_v1_header(mixed), b"PROXY UNKNOWN\r\n");
        assert_eq!(
            proxy_protocol_v2_header(mixed),
            b"\r\n\r\n\0\r\nQUIT\n\x21\0\0\0"
        );
    }

    #[test]
    fn routes_in_configuration_order_with_catch_all() {
        let routes = vec![
            RuleConfig {
                id: "exact".to_string(),
                name: "精确".to_string(),
                host: vec!["play.example.com".to_string()],
                backend: vec!["10.0.0.2:25565".to_string()],
                modify_virtual_host: false,
                enabled: true,
                ..RuleConfig::default()
            },
            RuleConfig {
                id: "wildcard".to_string(),
                name: "通配".to_string(),
                host: vec!["*.example.com".to_string()],
                backend: vec!["10.0.0.1:25565".to_string()],
                modify_virtual_host: false,
                enabled: true,
                ..RuleConfig::default()
            },
            RuleConfig {
                id: "default".to_string(),
                name: "兜底".to_string(),
                host: vec!["*".to_string()],
                backend: vec!["10.0.0.3:25565".to_string()],
                modify_virtual_host: false,
                enabled: true,
                ..RuleConfig::default()
            },
        ];
        assert_eq!(
            select_route(&routes, Some("play.example.com")).unwrap().id,
            "exact"
        );
        assert_eq!(
            select_route(&routes, Some("mod.example.com")).unwrap().id,
            "wildcard"
        );
        assert_eq!(
            select_route(&routes, Some("example.net")).unwrap().id,
            "default"
        );
    }

    #[tokio::test]
    async fn minecraft_status_probe_validates_status_and_pong() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let backend = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let handshake = read_packet(&mut stream, MAX_HANDSHAKE_PACKET)
                .await
                .unwrap();
            let parsed = parse_handshake(&handshake).unwrap();
            assert_eq!(parsed.hostname, "health.example.com");
            assert_eq!(parsed.protocol_version, 769);
            assert_eq!(parsed.next_state, 1);
            assert_eq!(
                packet_id(
                    &read_packet(&mut stream, MAX_HANDSHAKE_PACKET)
                        .await
                        .unwrap()
                ),
                Some(0)
            );
            let status = json!({
                "version": {"name": "1.21.4", "protocol": 769},
                "players": {"max": 20, "online": 0},
                "description": {"text": "ready"}
            });
            stream
                .write_all(&string_packet(0, &status.to_string()).unwrap())
                .await
                .unwrap();
            let ping = read_packet(&mut stream, MAX_HANDSHAKE_PACKET)
                .await
                .unwrap();
            stream.write_all(&ping).await.unwrap();
        });

        let mut stream = TcpStream::connect(address).await.unwrap();
        probe_minecraft_status(
            &mut stream,
            &address.to_string(),
            "health.example.com",
            769,
            ProxyProtocolVersion::Off,
        )
        .await
        .unwrap();
        backend.await.unwrap();
    }

    #[tokio::test]
    async fn minecraft_status_probe_rejects_non_minecraft_response() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let backend = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let _ = read_packet(&mut stream, MAX_HANDSHAKE_PACKET)
                .await
                .unwrap();
            let _ = read_packet(&mut stream, MAX_HANDSHAKE_PACKET)
                .await
                .unwrap();
            stream
                .write_all(&string_packet(0, "{}").unwrap())
                .await
                .unwrap();
        });

        let mut stream = TcpStream::connect(address).await.unwrap();
        let error = probe_minecraft_status(
            &mut stream,
            &address.to_string(),
            "health.example.com",
            769,
            ProxyProtocolVersion::Off,
        )
        .await
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        backend.await.unwrap();
    }
}
