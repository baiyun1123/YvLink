use std::{sync::Arc, time::Duration};

use mc_proxy::{
    AppConfig, CrossplayAuthType, ForwardConfig, HealthCheckMode, LoadBalancingStrategy, Metrics,
    ProxyProtocolVersion, RuleConfig, RuntimeManager, StatusConfig, StatusMode,
    StatusResponseConfig, create_listener, proxy_connection, serve,
};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
    time::timeout,
};

async fn echo_backend() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut data = Vec::new();
        stream.read_to_end(&mut data).await.unwrap();
        stream.write_all(&data).await.unwrap();
        stream.shutdown().await.unwrap();
    });
    (address, task)
}

async fn proxy_header_backend(
    version: ProxyProtocolVersion,
) -> (
    std::net::SocketAddr,
    oneshot::Receiver<Vec<u8>>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (header_tx, header_rx) = oneshot::channel();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let header = match version {
            ProxyProtocolVersion::V1 => {
                let mut header = Vec::new();
                loop {
                    header.push(stream.read_u8().await.unwrap());
                    if header.ends_with(b"\r\n") {
                        break;
                    }
                }
                header
            }
            ProxyProtocolVersion::V2 => {
                let mut header = vec![0_u8; 16];
                stream.read_exact(&mut header).await.unwrap();
                let address_length = usize::from(u16::from_be_bytes([header[14], header[15]]));
                header.resize(16 + address_length, 0);
                stream.read_exact(&mut header[16..]).await.unwrap();
                header
            }
            ProxyProtocolVersion::Off => Vec::new(),
        };
        header_tx.send(header).unwrap();
        let mut minecraft = Vec::new();
        stream.read_to_end(&mut minecraft).await.unwrap();
        stream.write_all(&minecraft).await.unwrap();
        stream.shutdown().await.unwrap();
    });
    (address, header_rx, task)
}

fn forward_config(backend: std::net::SocketAddr) -> ForwardConfig {
    let mut app = AppConfig::default();
    app.settings.listen = "127.0.0.1:0".parse().unwrap();
    app.rules[0].backend = vec![backend.to_string()];
    ForwardConfig::from_app(&app)
}

fn forward_config_with_backends(
    backends: Vec<std::net::SocketAddr>,
    strategy: LoadBalancingStrategy,
) -> ForwardConfig {
    let mut app = AppConfig::default();
    app.settings.listen = "127.0.0.1:0".parse().unwrap();
    app.rules[0].backend = backends
        .into_iter()
        .map(|backend| backend.to_string())
        .collect();
    app.rules[0].strategy = strategy;
    ForwardConfig::from_app(&app)
}

fn minecraft_handshake_for(hostname: &str, next_state: u8) -> Vec<u8> {
    minecraft_handshake_with_protocol(hostname, next_state, 761)
}

fn minecraft_handshake_with_protocol(hostname: &str, next_state: u8, protocol: usize) -> Vec<u8> {
    let mut payload = encode_varint(0);
    payload.extend_from_slice(&encode_varint(protocol));
    payload.extend_from_slice(&minecraft_string(hostname));
    payload.extend_from_slice(&[0x63, 0xdd]);
    payload.extend_from_slice(&encode_varint(usize::from(next_state)));
    let mut packet = encode_varint(payload.len());
    packet.extend(payload);
    packet
}

fn minecraft_handshake(hostname: &str) -> Vec<u8> {
    minecraft_handshake_for(hostname, 1)
}

fn minecraft_packet(id: u8, payload: &[u8]) -> Vec<u8> {
    let mut body = encode_varint(usize::from(id));
    body.extend_from_slice(payload);
    let mut packet = encode_varint(body.len());
    packet.extend_from_slice(&body);
    packet
}

fn encode_varint(mut value: usize) -> Vec<u8> {
    let mut output = Vec::with_capacity(5);
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        output.push(byte);
        if value == 0 {
            return output;
        }
    }
}

fn minecraft_string(value: &str) -> Vec<u8> {
    let mut encoded = encode_varint(value.len());
    encoded.extend_from_slice(value.as_bytes());
    encoded
}

fn decode_varint(bytes: &[u8], cursor: &mut usize) -> usize {
    let mut value = 0_usize;
    for shift in (0..35).step_by(7) {
        let byte = bytes[*cursor];
        *cursor += 1;
        value |= usize::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return value;
        }
    }
    panic!("VarInt too long")
}

async fn read_minecraft_packet(stream: &mut TcpStream) -> Vec<u8> {
    let mut packet = Vec::with_capacity(256);
    let mut length = 0_usize;
    for shift in (0..35).step_by(7) {
        let byte = stream.read_u8().await.unwrap();
        packet.push(byte);
        length |= usize::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            break;
        }
    }
    let header_length = packet.len();
    packet.resize(header_length + length, 0);
    stream
        .read_exact(&mut packet[header_length..])
        .await
        .unwrap();
    packet
}

async fn query_status(frontend: std::net::SocketAddr, host: &str) -> Value {
    let mut client = TcpStream::connect(frontend).await.unwrap();
    client.write_all(&minecraft_handshake(host)).await.unwrap();
    client.write_all(&minecraft_packet(0, &[])).await.unwrap();
    let ping_payload = 456_i64.to_be_bytes();
    client
        .write_all(&minecraft_packet(1, &ping_payload))
        .await
        .unwrap();
    let response = read_minecraft_packet(&mut client).await;
    let pong = read_minecraft_packet(&mut client).await;
    assert_eq!(pong, minecraft_packet(1, &ping_payload));

    let mut cursor = 0;
    let _packet_length = decode_varint(&response, &mut cursor);
    assert_eq!(decode_varint(&response, &mut cursor), 0);
    let string_length = decode_varint(&response, &mut cursor);
    serde_json::from_slice(&response[cursor..cursor + string_length]).unwrap()
}

#[tokio::test]
async fn routes_minecraft_handshake_by_hostname() {
    let (domain_backend, domain_task) = echo_backend().await;
    let frontend = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let frontend_address = frontend.local_addr().unwrap();
    let mut app = AppConfig::default();
    app.settings.listen = "127.0.0.1:0".parse().unwrap();
    app.settings.socket_buffer_bytes = 0;
    app.rules = vec![RuleConfig {
        id: "play".to_string(),
        name: "Play".to_string(),
        host: vec!["play.example.com".to_string()],
        backend: vec![domain_backend.to_string()],
        modify_virtual_host: false,
        enabled: true,
        ..RuleConfig::default()
    }];
    let config = ForwardConfig::from_app(&app);

    let proxy_task = tokio::spawn(async move {
        let (client, _) = frontend.accept().await.unwrap();
        proxy_connection(client, &config, Arc::new(Metrics::default()))
            .await
            .unwrap()
    });

    let handshake = minecraft_handshake("PLAY.EXAMPLE.COM");
    let mut client = TcpStream::connect(frontend_address).await.unwrap();
    client.write_all(&handshake).await.unwrap();
    client.shutdown().await.unwrap();
    let mut response = Vec::new();
    client.read_to_end(&mut response).await.unwrap();

    assert_eq!(response, handshake);
    let report = proxy_task.await.unwrap();
    assert_eq!(report.backend, domain_backend.to_string());
    assert_eq!(report.route_id, "play");
    domain_task.await.unwrap();
}

#[tokio::test]
async fn sends_proxy_protocol_v1_before_minecraft_handshake() {
    let (backend, header_rx, backend_task) = proxy_header_backend(ProxyProtocolVersion::V1).await;
    let frontend = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let frontend_address = frontend.local_addr().unwrap();
    let mut config = forward_config(backend);
    config.socket_buffer_bytes = 0;
    config.routes[0].proxy_protocol = ProxyProtocolVersion::V1;
    let metrics = Arc::new(Metrics::default());
    let observed_metrics = Arc::clone(&metrics);

    let proxy_task = tokio::spawn(async move {
        let (client, _) = frontend.accept().await.unwrap();
        proxy_connection(client, &config, metrics).await.unwrap()
    });

    let handshake = minecraft_handshake("v1.example.com");
    let mut client = TcpStream::connect(frontend_address).await.unwrap();
    let source = client.local_addr().unwrap();
    client.write_all(&handshake).await.unwrap();
    client.shutdown().await.unwrap();
    let mut response = Vec::new();
    client.read_to_end(&mut response).await.unwrap();

    let expected = format!(
        "PROXY TCP4 {} {} {} {}\r\n",
        source.ip(),
        frontend_address.ip(),
        source.port(),
        frontend_address.port()
    );
    assert_eq!(header_rx.await.unwrap(), expected.as_bytes());
    assert_eq!(response, handshake);
    assert_eq!(observed_metrics.snapshot().proxy_protocol_v1_headers, 1);
    proxy_task.await.unwrap();
    backend_task.await.unwrap();
}

#[tokio::test]
async fn sends_proxy_protocol_v2_before_minecraft_handshake() {
    let (backend, header_rx, backend_task) = proxy_header_backend(ProxyProtocolVersion::V2).await;
    let frontend = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let frontend_address = frontend.local_addr().unwrap();
    let mut config = forward_config(backend);
    config.socket_buffer_bytes = 0;
    config.routes[0].proxy_protocol = ProxyProtocolVersion::V2;
    let metrics = Arc::new(Metrics::default());
    let observed_metrics = Arc::clone(&metrics);

    let proxy_task = tokio::spawn(async move {
        let (client, _) = frontend.accept().await.unwrap();
        proxy_connection(client, &config, metrics).await.unwrap()
    });

    let handshake = minecraft_handshake("v2.example.com");
    let mut client = TcpStream::connect(frontend_address).await.unwrap();
    let source = client.local_addr().unwrap();
    client.write_all(&handshake).await.unwrap();
    client.shutdown().await.unwrap();
    let mut response = Vec::new();
    client.read_to_end(&mut response).await.unwrap();

    let header = header_rx.await.unwrap();
    assert_eq!(&header[..16], b"\r\n\r\n\0\r\nQUIT\n\x21\x11\0\x0c");
    assert_eq!(
        &header[16..20],
        &source
            .ip()
            .to_string()
            .parse::<std::net::Ipv4Addr>()
            .unwrap()
            .octets()
    );
    assert_eq!(
        &header[20..24],
        &frontend_address
            .ip()
            .to_string()
            .parse::<std::net::Ipv4Addr>()
            .unwrap()
            .octets()
    );
    assert_eq!(&header[24..26], &source.port().to_be_bytes());
    assert_eq!(&header[26..28], &frontend_address.port().to_be_bytes());
    assert_eq!(response, handshake);
    assert_eq!(observed_metrics.snapshot().proxy_protocol_v2_headers, 1);
    proxy_task.await.unwrap();
    backend_task.await.unwrap();
}

#[tokio::test]
async fn fails_over_to_next_backend_and_reports_health() {
    let unavailable = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let unavailable_address = unavailable.local_addr().unwrap();
    drop(unavailable);
    let (healthy_address, healthy_task) = echo_backend().await;
    let frontend = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let frontend_address = frontend.local_addr().unwrap();
    let mut config = forward_config_with_backends(
        vec![unavailable_address, healthy_address],
        LoadBalancingStrategy::Sequential,
    );
    config.socket_buffer_bytes = 0;
    let metrics = Arc::new(Metrics::default());
    let observed_metrics = Arc::clone(&metrics);
    let observed_config = config.clone();

    let proxy_task = tokio::spawn(async move {
        let (client, _) = frontend.accept().await.unwrap();
        proxy_connection(client, &config, metrics).await.unwrap()
    });

    let handshake = minecraft_handshake("pool.example.com");
    let mut client = TcpStream::connect(frontend_address).await.unwrap();
    client.write_all(&handshake).await.unwrap();
    client.shutdown().await.unwrap();
    let mut response = Vec::new();
    client.read_to_end(&mut response).await.unwrap();

    assert_eq!(response, handshake);
    let report = proxy_task.await.unwrap();
    assert_eq!(report.backend, healthy_address.to_string());
    assert_eq!(observed_metrics.snapshot().backend_attempt_failures, 1);
    assert_eq!(observed_metrics.snapshot().backend_failovers, 1);
    let health = observed_config.backend_health("default");
    assert_eq!(health[0].failed_attempts, 1);
    assert_eq!(health[1].successful_connections, 1);
    assert_eq!(health[1].active_connections, 0);
    healthy_task.await.unwrap();
}

#[tokio::test]
async fn managed_status_fails_over_between_backends() {
    let unavailable = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let unavailable_address = unavailable.local_addr().unwrap();
    drop(unavailable);
    let backend_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let healthy_address = backend_listener.local_addr().unwrap();
    let backend_task = tokio::spawn(async move {
        let (mut stream, _) = backend_listener.accept().await.unwrap();
        let _handshake = read_minecraft_packet(&mut stream).await;
        let _request = read_minecraft_packet(&mut stream).await;
        let response = json!({
            "version": {"name": "Fabric", "protocol": 767},
            "players": {"max": 40, "online": 7},
            "description": {"text": "healthy"},
            "forgeData": {"mods": []}
        })
        .to_string();
        stream
            .write_all(&minecraft_packet(0, &minecraft_string(&response)))
            .await
            .unwrap();
    });
    let frontend = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let frontend_address = frontend.local_addr().unwrap();
    let mut config = forward_config_with_backends(
        vec![unavailable_address, healthy_address],
        LoadBalancingStrategy::Sequential,
    );
    config.socket_buffer_bytes = 0;
    config.routes[0].status = Some(StatusConfig {
        mode: StatusMode::Backend,
        cache_ttl_secs: 60,
        motd: None,
        version_name: None,
        protocol: None,
        online: None,
        max: None,
        fallback: None,
    });
    let metrics = Arc::new(Metrics::default());
    let observed_metrics = Arc::clone(&metrics);

    let proxy_task = tokio::spawn(async move {
        let (client, _) = frontend.accept().await.unwrap();
        proxy_connection(client, &config, metrics).await.unwrap()
    });
    let response = query_status(frontend_address, "pool.example.com").await;

    assert_eq!(response["version"]["name"], "Fabric");
    assert_eq!(response["forgeData"]["mods"], json!([]));
    proxy_task.await.unwrap();
    backend_task.await.unwrap();
    assert_eq!(observed_metrics.snapshot().backend_attempt_failures, 1);
    assert_eq!(observed_metrics.snapshot().backend_failovers, 1);
    assert_eq!(observed_metrics.snapshot().backend_failures, 0);
}

#[tokio::test]
async fn serves_custom_status_without_connecting_backend() {
    let backend_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let backend = backend_listener.local_addr().unwrap();
    let frontend = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let frontend_address = frontend.local_addr().unwrap();
    let mut config = forward_config(backend);
    config.socket_buffer_bytes = 0;
    config.routes[0].status = Some(StatusConfig {
        motd: Some("§a代理自定义 MOTD".to_string()),
        version_name: Some("原版 1.21".to_string()),
        protocol: Some(767),
        online: Some(12),
        max: Some(100),
        ..StatusConfig::default()
    });
    let metrics = Arc::new(Metrics::default());
    let observed_metrics = Arc::clone(&metrics);

    let proxy_task = tokio::spawn(async move {
        let (client, _) = frontend.accept().await.unwrap();
        proxy_connection(client, &config, metrics).await.unwrap()
    });

    let mut client = TcpStream::connect(frontend_address).await.unwrap();
    client
        .write_all(&minecraft_handshake("status.example.com"))
        .await
        .unwrap();
    client.write_all(&minecraft_packet(0, &[])).await.unwrap();
    let ping_payload = 123_i64.to_be_bytes();
    client
        .write_all(&minecraft_packet(1, &ping_payload))
        .await
        .unwrap();

    let status_response = read_minecraft_packet(&mut client).await;
    let pong = read_minecraft_packet(&mut client).await;
    assert!(String::from_utf8_lossy(&status_response).contains("代理自定义 MOTD"));
    assert!(String::from_utf8_lossy(&status_response).contains("原版 1.21"));
    assert_eq!(pong, minecraft_packet(1, &ping_payload));

    let report = proxy_task.await.unwrap();
    assert_eq!(report.backend, "<local-status>");
    assert_eq!(observed_metrics.snapshot().local_status_responses, 1);
    assert!(
        timeout(Duration::from_millis(100), backend_listener.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn backend_status_overlay_preserves_modded_fields_and_uses_cache() {
    let backend_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let backend = backend_listener.local_addr().unwrap();
    let backend_response = json!({
        "version": {"name": "NeoForge 21.1", "protocol": 767},
        "players": {"max": 80, "online": 23, "sample": []},
        "description": {"text": "后端原始 MOTD"},
        "favicon": "data:image/png;base64,TEST",
        "forgeData": {"mods": [{"modId": "example", "modmarker": "1.0"}]},
        "unknownExtension": {"kept": true}
    })
    .to_string();
    let (header_tx, header_rx) = oneshot::channel();
    let backend_task = tokio::spawn(async move {
        let (mut stream, _) = backend_listener.accept().await.unwrap();
        let mut header = vec![0_u8; 16];
        stream.read_exact(&mut header).await.unwrap();
        let address_length = usize::from(u16::from_be_bytes([header[14], header[15]]));
        header.resize(16 + address_length, 0);
        stream.read_exact(&mut header[16..]).await.unwrap();
        header_tx.send(header).unwrap();
        let _handshake = read_minecraft_packet(&mut stream).await;
        let _request = read_minecraft_packet(&mut stream).await;
        stream
            .write_all(&minecraft_packet(0, &minecraft_string(&backend_response)))
            .await
            .unwrap();
    });

    let frontend = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let frontend_address = frontend.local_addr().unwrap();
    let mut config = forward_config(backend);
    config.socket_buffer_bytes = 0;
    config.routes[0].proxy_protocol = ProxyProtocolVersion::V2;
    config.routes[0].status = Some(StatusConfig {
        mode: StatusMode::Backend,
        cache_ttl_secs: 60,
        motd: Some("§b代理覆盖 MOTD".to_string()),
        version_name: None,
        protocol: None,
        online: None,
        max: None,
        fallback: None,
    });
    let metrics = Arc::new(Metrics::default());
    let observed_metrics = Arc::clone(&metrics);
    let proxy_task = tokio::spawn(async move {
        for _ in 0..2 {
            let (client, _) = frontend.accept().await.unwrap();
            proxy_connection(client, &config, Arc::clone(&metrics))
                .await
                .unwrap();
        }
    });

    let first = query_status(frontend_address, "mod.example.com").await;
    let second = query_status(frontend_address, "mod.example.com").await;
    assert_eq!(first, second);
    assert_eq!(first["description"]["text"], "§b代理覆盖 MOTD");
    assert_eq!(first["version"]["name"], "NeoForge 21.1");
    assert_eq!(first["players"]["online"], 23);
    assert_eq!(first["forgeData"]["mods"][0]["modId"], "example");
    assert_eq!(first["favicon"], "data:image/png;base64,TEST");
    assert_eq!(first["unknownExtension"]["kept"], true);
    assert_eq!(observed_metrics.snapshot().status_cache_hits, 1);
    assert_eq!(
        observed_metrics.snapshot().proxy_protocol_v2_headers,
        1,
        "缓存命中不应再次连接后端或发送 PROXY 头"
    );
    let header = header_rx.await.unwrap();
    assert_eq!(&header[..16], b"\r\n\r\n\0\r\nQUIT\n\x21\x11\0\x0c");

    proxy_task.await.unwrap();
    backend_task.await.unwrap();
}

#[tokio::test]
async fn returns_configured_status_fallback_when_backend_is_offline() {
    let unavailable = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let backend = unavailable.local_addr().unwrap();
    drop(unavailable);
    let frontend = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let frontend_address = frontend.local_addr().unwrap();
    let mut config = forward_config(backend);
    config.socket_buffer_bytes = 0;
    config.routes[0].status = Some(StatusConfig {
        mode: StatusMode::Backend,
        cache_ttl_secs: -1,
        motd: None,
        version_name: None,
        protocol: None,
        online: None,
        max: None,
        fallback: Some(StatusResponseConfig {
            motd: Some("§c维护中，请稍后再试".to_string()),
            version_name: Some("后端离线".to_string()),
            protocol: Some(-1),
            online: Some(0),
            max: Some(100),
        }),
    });
    let metrics = Arc::new(Metrics::default());
    let observed_metrics = Arc::clone(&metrics);
    let proxy_task = tokio::spawn(async move {
        let (client, _) = frontend.accept().await.unwrap();
        proxy_connection(client, &config, metrics).await.unwrap()
    });

    let response = query_status(frontend_address, "offline.example.com").await;
    assert_eq!(response["description"]["text"], "§c维护中，请稍后再试");
    assert_eq!(response["version"]["name"], "后端离线");
    assert_eq!(response["version"]["protocol"], -1);
    assert_eq!(observed_metrics.snapshot().status_fallbacks, 1);
    assert_eq!(observed_metrics.snapshot().backend_failures, 1);
    proxy_task.await.unwrap();
}

#[tokio::test]
async fn rejects_player_not_in_route_whitelist_before_backend() {
    let backend_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let backend = backend_listener.local_addr().unwrap();
    let frontend = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let frontend_address = frontend.local_addr().unwrap();
    let mut config = forward_config(backend);
    config.socket_buffer_bytes = 0;
    config.routes[0].whitelist_enabled = true;
    config.routes[0].whitelist = vec!["Alice".to_string()];
    config.routes[0].whitelist_message = "§c仅限白名单玩家".to_string();
    let metrics = Arc::new(Metrics::default());
    let observed_metrics = Arc::clone(&metrics);

    let proxy_task = tokio::spawn(async move {
        let (client, _) = frontend.accept().await.unwrap();
        proxy_connection(client, &config, metrics).await.unwrap()
    });

    let mut client = TcpStream::connect(frontend_address).await.unwrap();
    client
        .write_all(&minecraft_handshake_for("play.example.com", 2))
        .await
        .unwrap();
    client
        .write_all(&minecraft_packet(0, &[3, b'B', b'o', b'b']))
        .await
        .unwrap();
    let disconnect = read_minecraft_packet(&mut client).await;
    assert!(String::from_utf8_lossy(&disconnect).contains("仅限白名单玩家"));

    let report = proxy_task.await.unwrap();
    assert_eq!(report.backend, "<whitelist-denied>");
    assert_eq!(observed_metrics.snapshot().whitelist_denials, 1);
    assert!(
        timeout(Duration::from_millis(100), backend_listener.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn whitelist_allows_player_and_preserves_modded_handshake_bytes() {
    let (backend, backend_task) = echo_backend().await;
    let frontend = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let frontend_address = frontend.local_addr().unwrap();
    let mut config = forward_config(backend);
    config.socket_buffer_bytes = 0;
    config.routes[0].whitelist_enabled = true;
    config.routes[0].whitelist = vec!["Alice".to_string()];

    let proxy_task = tokio::spawn(async move {
        let (client, _) = frontend.accept().await.unwrap();
        proxy_connection(client, &config, Arc::new(Metrics::default()))
            .await
            .unwrap()
    });

    let handshake = minecraft_handshake_for("mod.example.com\0FML3\0", 2);
    let login = minecraft_packet(0, &[5, b'A', b'l', b'i', b'c', b'e']);
    let plugin_data = minecraft_packet(2, b"forge:handshake");
    let mut client = TcpStream::connect(frontend_address).await.unwrap();
    client.write_all(&handshake).await.unwrap();
    client.write_all(&login).await.unwrap();
    client.write_all(&plugin_data).await.unwrap();
    client.shutdown().await.unwrap();

    let mut response = Vec::new();
    client.read_to_end(&mut response).await.unwrap();
    let expected = [handshake, login, plugin_data].concat();
    assert_eq!(response, expected);
    proxy_task.await.unwrap();
    backend_task.await.unwrap();
}

#[tokio::test]
async fn preserves_fabric_forge_and_neoforge_negotiation_matrix_bidirectionally() {
    struct Fixture {
        name: &'static str,
        protocol: usize,
        marker: &'static str,
        server_payload: &'static [u8],
        client_payload: &'static [u8],
        metric_field: &'static str,
    }

    let fixtures = [
        Fixture {
            name: "Fabric 1.20.1 login/custom payload",
            protocol: 763,
            marker: "",
            server_payload: b"fabric:registry/sync\0server-registry-bytes",
            client_payload: b"fabric:registry/sync\0client-ack-bytes",
            metric_field: "unmarked",
        },
        Fixture {
            name: "Forge 1.12.2 legacy FML handshake",
            protocol: 340,
            marker: "\0FML\0",
            server_payload: b"FML|HS\0ServerHello\0mod-list",
            client_payload: b"FML|HS\0ClientHello\0mod-list",
            metric_field: "legacy",
        },
        Fixture {
            name: "Forge 1.16.5 FML2 LoginPluginMessage",
            protocol: 754,
            marker: "\0FML2\0",
            server_payload: b"fml:loginwrapper\0ModList\0registry",
            client_payload: b"fml:loginwrapper\0Ack\0registry",
            metric_field: "modern-login",
        },
        Fixture {
            name: "Forge 1.20.1 FML3 LoginPluginMessage",
            protocol: 763,
            marker: "\0FML3\0",
            server_payload: b"fml:loginwrapper\0ConfigData\0\x00\xff\x80",
            client_payload: b"fml:loginwrapper\0ConfigAck\0\xff\x00\x7f",
            metric_field: "modern-login",
        },
        Fixture {
            name: "NeoForge 1.21.1 Configuration custom payload",
            protocol: 767,
            marker: "\0FORGE",
            server_payload: b"neoforge:network\0configuration\0register\0\x00\xfe",
            client_payload: b"neoforge:network\0configuration\0ack\0\xff\x01",
            metric_field: "configuration",
        },
        Fixture {
            name: "Forge NAT marker Configuration custom payload",
            protocol: 767,
            marker: "\0FORGE2",
            server_payload: b"forge:network\0configuration\0version",
            client_payload: b"forge:network\0configuration\0accepted",
            metric_field: "configuration",
        },
    ];

    for fixture in fixtures {
        let backend_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let backend_address = backend_listener.local_addr().unwrap();
        let client_host = format!("mod.example.com{}", fixture.marker);
        let backend_host = format!("127.0.0.1{}", fixture.marker);
        let client_handshake = minecraft_handshake_with_protocol(&client_host, 2, fixture.protocol);
        let backend_handshake =
            minecraft_handshake_with_protocol(&backend_host, 2, fixture.protocol);
        let login_start = minecraft_packet(0, &minecraft_string("Alice"));
        let server_challenge = minecraft_packet(4, fixture.server_payload);
        let client_response = minecraft_packet(2, fixture.client_payload);
        let server_finish = minecraft_packet(3, b"negotiation-complete");
        let expected_prefix = [backend_handshake, login_start.clone()].concat();
        let expected_response = client_response.clone();
        let sent_challenge = server_challenge.clone();
        let sent_finish = server_finish.clone();

        let backend_task = tokio::spawn(async move {
            let (mut stream, _) = backend_listener.accept().await.unwrap();
            let mut prefix = vec![0; expected_prefix.len()];
            stream.read_exact(&mut prefix).await.unwrap();
            assert_eq!(prefix, expected_prefix);
            stream.write_all(&sent_challenge).await.unwrap();
            let mut response = vec![0; expected_response.len()];
            stream.read_exact(&mut response).await.unwrap();
            assert_eq!(response, expected_response);
            stream.write_all(&sent_finish).await.unwrap();
            stream.shutdown().await.unwrap();
        });

        let frontend = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let frontend_address = frontend.local_addr().unwrap();
        let mut config = forward_config(backend_address);
        config.socket_buffer_bytes = 0;
        config.routes[0].modify_virtual_host = true;
        let metrics = Arc::new(Metrics::default());
        let observed_metrics = Arc::clone(&metrics);
        let proxy_task = tokio::spawn(async move {
            let (client, _) = frontend.accept().await.unwrap();
            proxy_connection(client, &config, metrics).await.unwrap()
        });

        let mut client = TcpStream::connect(frontend_address).await.unwrap();
        client.write_all(&client_handshake).await.unwrap();
        client.write_all(&login_start).await.unwrap();
        let mut challenge = vec![0; server_challenge.len()];
        client.read_exact(&mut challenge).await.unwrap();
        assert_eq!(challenge, server_challenge, "{}", fixture.name);
        client.write_all(&client_response).await.unwrap();
        client.shutdown().await.unwrap();
        let mut finish = Vec::new();
        client.read_to_end(&mut finish).await.unwrap();
        assert_eq!(finish, server_finish, "{}", fixture.name);

        let report = proxy_task.await.unwrap();
        assert_eq!(report.backend, backend_address.to_string());
        backend_task.await.unwrap();

        let snapshot = observed_metrics.snapshot();
        match fixture.metric_field {
            "unmarked" => assert_eq!(snapshot.unmarked_handshakes, 1),
            "legacy" => assert_eq!(snapshot.legacy_forge_handshakes, 1),
            "modern-login" => assert_eq!(snapshot.modern_forge_login_handshakes, 1),
            "configuration" => assert_eq!(snapshot.configuration_forge_handshakes, 1),
            _ => unreachable!(),
        }
    }
}

#[tokio::test]
async fn forwards_large_payload_and_preserves_half_close() {
    let (backend, backend_task) = echo_backend().await;
    let frontend = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let frontend_address = frontend.local_addr().unwrap();
    let mut config = forward_config(backend);
    config.copy_buffer_bytes = 16 * 1024;
    config.socket_buffer_bytes = 0;
    let metrics = Arc::new(Metrics::default());
    let observed_metrics = Arc::clone(&metrics);

    let proxy_task = tokio::spawn(async move {
        let (client, _) = frontend.accept().await.unwrap();
        proxy_connection(client, &config, metrics).await.unwrap()
    });

    let handshake = minecraft_handshake("anything.example.com");
    let payload = vec![0x5a; 256 * 1024 + 137];
    let mut client = TcpStream::connect(frontend_address).await.unwrap();
    client.write_all(&handshake).await.unwrap();
    client.write_all(&payload).await.unwrap();

    timeout(Duration::from_secs(1), async {
        loop {
            if observed_metrics.snapshot().upload_bytes == (handshake.len() + payload.len()) as u64
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("live upload metrics were not updated");

    client.shutdown().await.unwrap();
    let mut response = Vec::new();
    timeout(Duration::from_secs(3), client.read_to_end(&mut response))
        .await
        .expect("proxy did not preserve half-close")
        .unwrap();

    let mut expected = handshake.clone();
    expected.extend_from_slice(&payload);
    assert_eq!(response, expected);
    let report = proxy_task.await.unwrap();
    assert_eq!(report.upload_bytes, expected.len() as u64);
    assert_eq!(report.download_bytes, expected.len() as u64);
    assert_eq!(
        observed_metrics.snapshot().download_bytes,
        expected.len() as u64
    );
    backend_task.await.unwrap();
}

#[tokio::test]
async fn reports_backend_connection_failure() {
    let unavailable = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let backend = unavailable.local_addr().unwrap();
    drop(unavailable);

    let pair_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let pair_address = pair_listener.local_addr().unwrap();
    let connector = tokio::spawn(async move { TcpStream::connect(pair_address).await.unwrap() });
    let (proxy_side, _) = pair_listener.accept().await.unwrap();
    let mut client_side = connector.await.unwrap();

    let mut config = forward_config(backend);
    config.connect_timeout_ms = 200;
    config.socket_buffer_bytes = 0;
    client_side
        .write_all(&minecraft_handshake("anything.example.com"))
        .await
        .unwrap();
    let error = proxy_connection(proxy_side, &config, Arc::new(Metrics::default()))
        .await
        .unwrap_err();
    assert!(error.is_backend_failure());
}

#[tokio::test]
async fn server_rejects_connections_over_limit_and_shuts_down() {
    let backend_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let backend = backend_listener.local_addr().unwrap();
    let backend_task = tokio::spawn(async move {
        let (_stream, _) = backend_listener.accept().await.unwrap();
        tokio::time::sleep(Duration::from_secs(5)).await;
    });

    let mut config = forward_config(backend);
    config.max_connections = 1;
    config.socket_buffer_bytes = 0;
    config.shutdown_grace_secs = 1;
    config.stats_interval_secs = 60;
    let listener = create_listener(&config).unwrap();
    let frontend = listener.local_addr().unwrap();
    let config = Arc::new(config);
    let metrics = Arc::new(Metrics::default());
    let observed_metrics = Arc::clone(&metrics);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    let server_task = tokio::spawn(serve(listener, config, metrics, async move {
        let _ = shutdown_rx.await;
    }));

    let first = TcpStream::connect(frontend).await.unwrap();
    timeout(Duration::from_secs(1), async {
        loop {
            if observed_metrics.snapshot().active_connections == 1 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();

    let mut second = TcpStream::connect(frontend).await.unwrap();
    timeout(Duration::from_secs(1), async {
        loop {
            if observed_metrics.snapshot().rejected_connections == 1 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let mut byte = [0_u8; 1];
    let read = timeout(Duration::from_secs(1), second.read(&mut byte))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(read, 0);

    shutdown_tx.send(()).unwrap();
    timeout(Duration::from_secs(2), server_task)
        .await
        .expect("server did not shut down")
        .unwrap()
        .unwrap();
    drop(first);
    backend_task.abort();
}

#[tokio::test]
async fn active_health_check_runs_without_player_connections() {
    let backend = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let backend_address = backend.local_addr().unwrap();
    let mut app = AppConfig::default();
    app.settings.listen = "127.0.0.1:0".parse().unwrap();
    app.settings.socket_buffer_bytes = 0;
    app.rules[0].backend = vec![backend_address.to_string()];
    app.rules[0].health_check.enabled = true;
    app.rules[0].health_check.interval_secs = 1;
    app.rules[0].health_check.timeout_ms = 500;
    app.rules[0].health_check.healthy_threshold = 1;
    let config = Arc::new(ForwardConfig::from_app(&app));
    let observed_config = Arc::clone(&config);
    let listener = create_listener(&config).unwrap();
    let metrics = Arc::new(Metrics::default());
    let observed_metrics = Arc::clone(&metrics);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let server_task = tokio::spawn(serve(listener, config, metrics, async move {
        let _ = shutdown_rx.await;
    }));

    timeout(Duration::from_secs(2), async {
        loop {
            if observed_metrics.snapshot().health_check_successes >= 1 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let health = observed_config.backend_health("default");
    assert_eq!(health[0].health, mc_proxy::BackendHealthState::Healthy);
    assert_eq!(health[0].health_check_successes, 1);

    shutdown_tx.send(()).unwrap();
    timeout(Duration::from_secs(2), server_task)
        .await
        .expect("server did not shut down")
        .unwrap()
        .unwrap();
    drop(backend);
}

#[tokio::test]
async fn minecraft_status_health_check_requires_valid_application_response() {
    let backend = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let backend_address = backend.local_addr().unwrap();
    let backend_task = tokio::spawn(async move {
        let (mut stream, _) = backend.accept().await.unwrap();
        let handshake = read_minecraft_packet(&mut stream).await;
        let mut cursor = 0;
        let _ = decode_varint(&handshake, &mut cursor);
        assert_eq!(decode_varint(&handshake, &mut cursor), 0);
        assert_eq!(decode_varint(&handshake, &mut cursor), 769);
        let host_length = decode_varint(&handshake, &mut cursor);
        assert_eq!(
            std::str::from_utf8(&handshake[cursor..cursor + host_length]).unwrap(),
            "health.fixture.local"
        );
        let request = read_minecraft_packet(&mut stream).await;
        assert_eq!(request, minecraft_packet(0, &[]));
        let response = json!({
            "version": {"name": "fixture", "protocol": 769},
            "players": {"max": 20, "online": 0},
            "description": {"text": "ready"}
        });
        stream
            .write_all(&minecraft_packet(
                0,
                &minecraft_string(&response.to_string()),
            ))
            .await
            .unwrap();
        let ping = read_minecraft_packet(&mut stream).await;
        stream.write_all(&ping).await.unwrap();
    });

    let mut app = AppConfig::default();
    app.settings.listen = "127.0.0.1:0".parse().unwrap();
    app.settings.socket_buffer_bytes = 0;
    app.rules[0].host = vec!["health.fixture.local".to_string()];
    app.rules[0].backend = vec![backend_address.to_string()];
    app.rules[0].health_check.enabled = true;
    app.rules[0].health_check.mode = HealthCheckMode::MinecraftStatus;
    app.rules[0].health_check.interval_secs = 1;
    app.rules[0].health_check.timeout_ms = 500;
    app.rules[0].health_check.healthy_threshold = 1;
    let config = Arc::new(ForwardConfig::from_app(&app));
    let observed_config = Arc::clone(&config);
    let listener = create_listener(&config).unwrap();
    let metrics = Arc::new(Metrics::default());
    let observed_metrics = Arc::clone(&metrics);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let server_task = tokio::spawn(serve(listener, config, metrics, async move {
        let _ = shutdown_rx.await;
    }));

    timeout(Duration::from_secs(2), async {
        loop {
            if observed_metrics.snapshot().health_check_successes >= 1 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let health = observed_config.backend_health("default");
    assert_eq!(health[0].health, mc_proxy::BackendHealthState::Healthy);
    backend_task.await.unwrap();

    shutdown_tx.send(()).unwrap();
    timeout(Duration::from_secs(2), server_task)
        .await
        .expect("server did not shut down")
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn minecraft_status_health_check_rejects_reachable_non_minecraft_port() {
    let backend = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let backend_address = backend.local_addr().unwrap();
    let backend_task = tokio::spawn(async move {
        let (mut stream, _) = backend.accept().await.unwrap();
        let _ = read_minecraft_packet(&mut stream).await;
        let _ = read_minecraft_packet(&mut stream).await;
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
            .await
            .unwrap();
        stream.shutdown().await.unwrap();
    });

    let mut app = AppConfig::default();
    app.settings.listen = "127.0.0.1:0".parse().unwrap();
    app.settings.socket_buffer_bytes = 0;
    app.rules[0].host = vec!["health.fixture.local".to_string()];
    app.rules[0].backend = vec![backend_address.to_string()];
    app.rules[0].health_check.enabled = true;
    app.rules[0].health_check.mode = HealthCheckMode::MinecraftStatus;
    app.rules[0].health_check.interval_secs = 1;
    app.rules[0].health_check.timeout_ms = 500;
    app.rules[0].health_check.unhealthy_threshold = 1;
    let config = Arc::new(ForwardConfig::from_app(&app));
    let observed_config = Arc::clone(&config);
    let listener = create_listener(&config).unwrap();
    let metrics = Arc::new(Metrics::default());
    let observed_metrics = Arc::clone(&metrics);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let server_task = tokio::spawn(serve(listener, config, metrics, async move {
        let _ = shutdown_rx.await;
    }));

    timeout(Duration::from_secs(2), async {
        loop {
            if observed_metrics.snapshot().health_check_failures >= 1 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let health = observed_config.backend_health("default");
    assert_eq!(health[0].health, mc_proxy::BackendHealthState::Unhealthy);
    backend_task.await.unwrap();

    shutdown_tx.send(()).unwrap();
    timeout(Duration::from_secs(2), server_task)
        .await
        .expect("server did not shut down")
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn manager_persists_rule_changes() {
    let temporary = std::env::temp_dir().join(format!(
        "mc-proxy-manager-test-{}-{}.toml",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut config = AppConfig::default();
    config.settings.proxy_enabled = false;
    let manager = RuntimeManager::new(config, temporary.clone());
    manager.start().await.unwrap();

    let mut rule = RuleConfig {
        id: "secondary".to_string(),
        name: "备用线路".to_string(),
        host: vec!["secondary.example.com".to_string()],
        backend: vec!["127.0.0.1:25567".to_string()],
        modify_virtual_host: false,
        enabled: false,
        ..RuleConfig::default()
    };
    rule.health_check.enabled = true;
    rule.health_check.mode = HealthCheckMode::MinecraftStatus;
    manager.create_rule(rule).await.unwrap();
    let mut crossplay = manager.config().await.crossplay;
    crossplay.auth_type = CrossplayAuthType::Floodgate;
    manager.update_crossplay(crossplay).await.unwrap();

    let applied = manager.config().await;
    assert_eq!(applied.rules[0].id, "secondary");
    assert_eq!(applied.rules[1].id, "default");
    let persisted = std::fs::read_to_string(&temporary).unwrap();
    assert!(persisted.contains("secondary"));
    assert!(persisted.contains("[[rules]]"));
    assert!(persisted.contains("[rules.health_check]"));
    assert!(persisted.contains("mode = \"minecraft-status\""));
    assert!(persisted.contains("interval_secs = 30"));
    assert!(persisted.contains("auth_type = \"floodgate\""));
    manager.shutdown().await;
    std::fs::remove_file(temporary).unwrap();
}
