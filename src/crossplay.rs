use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use tokio::{net::UdpSocket, time::timeout};

use crate::{CrossplayAuthType, CrossplayConfig};

const RAKNET_MAGIC: [u8; 16] = [
    0x00, 0xff, 0xff, 0x00, 0xfe, 0xfe, 0xfe, 0xfe, 0xfd, 0xfd, 0xfd, 0xfd, 0x12, 0x34, 0x56, 0x78,
];
const PROBE_TIMEOUT: Duration = Duration::from_millis(800);

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct CrossplayStatus {
    pub enabled: bool,
    pub online: bool,
    pub bedrock_listen: SocketAddr,
    pub java_target: String,
    pub auth_type: CrossplayAuthType,
    pub latency_ms: Option<u64>,
    pub motd: Option<String>,
    pub error: Option<String>,
}

pub async fn crossplay_status(config: &CrossplayConfig) -> CrossplayStatus {
    let mut status = CrossplayStatus {
        enabled: config.enabled,
        online: false,
        bedrock_listen: config.bedrock_listen,
        java_target: format!("{}:{}", config.java_address, config.java_port),
        auth_type: config.auth_type,
        latency_ms: None,
        motd: None,
        error: None,
    };
    if !config.enabled {
        return status;
    }

    match probe_geyser(config.bedrock_listen).await {
        Ok((latency, motd)) => {
            status.online = true;
            status.latency_ms = Some(latency.as_millis().min(u128::from(u64::MAX)) as u64);
            status.motd = Some(motd);
        }
        Err(error) => status.error = Some(error),
    }
    status
}

async fn probe_geyser(listen: SocketAddr) -> Result<(Duration, String), String> {
    let target = probe_target(listen);
    let bind = match target {
        SocketAddr::V4(_) => SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0),
        SocketAddr::V6(_) => SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 0),
    };
    let socket = UdpSocket::bind(bind)
        .await
        .map_err(|error| format!("无法创建 Bedrock UDP 探针: {error}"))?;
    let packet = unconnected_ping();
    let started = Instant::now();
    socket
        .send_to(&packet, target)
        .await
        .map_err(|error| format!("无法发送 Bedrock UDP 探针: {error}"))?;
    let mut response = [0_u8; 2048];
    let (length, _) = timeout(PROBE_TIMEOUT, socket.recv_from(&mut response))
        .await
        .map_err(|_| "Geyser UDP 探针超时".to_string())?
        .map_err(|error| format!("无法接收 Geyser UDP 响应: {error}"))?;
    let motd = parse_unconnected_pong(&response[..length])
        .ok_or_else(|| "Geyser 返回了无效的 RakNet Pong".to_string())?;
    Ok((started.elapsed(), motd.to_string()))
}

fn probe_target(listen: SocketAddr) -> SocketAddr {
    match listen.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => {
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), listen.port())
        }
        IpAddr::V6(ip) if ip.is_unspecified() => {
            SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), listen.port())
        }
        _ => listen,
    }
}

fn unconnected_ping() -> Vec<u8> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64;
    let guid = timestamp ^ 0x4d43_5245_4c41_5900_i64;
    let mut packet = Vec::with_capacity(33);
    packet.push(0x01);
    packet.extend_from_slice(&timestamp.to_be_bytes());
    packet.extend_from_slice(&RAKNET_MAGIC);
    packet.extend_from_slice(&guid.to_be_bytes());
    packet
}

fn parse_unconnected_pong(packet: &[u8]) -> Option<&str> {
    if packet.len() < 35 || packet[0] != 0x1c || packet[17..33] != RAKNET_MAGIC {
        return None;
    }
    let length = usize::from(u16::from_be_bytes([packet[33], packet[34]]));
    let end = 35_usize.checked_add(length)?;
    std::str::from_utf8(packet.get(35..end)?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_raknet_unconnected_pong() {
        let motd = "MCPE;Crossplay;900;1.21;2;20;id;world;Survival;1;19132;19133";
        let mut packet = vec![0x1c];
        packet.extend_from_slice(&0_i64.to_be_bytes());
        packet.extend_from_slice(&0_i64.to_be_bytes());
        packet.extend_from_slice(&RAKNET_MAGIC);
        packet.extend_from_slice(&(motd.len() as u16).to_be_bytes());
        packet.extend_from_slice(motd.as_bytes());
        assert_eq!(parse_unconnected_pong(&packet), Some(motd));
    }

    #[test]
    fn unspecified_probe_targets_loopback() {
        assert_eq!(
            probe_target("0.0.0.0:19132".parse().unwrap()),
            "127.0.0.1:19132".parse().unwrap()
        );
    }

    #[tokio::test]
    async fn detects_live_geyser_compatible_udp_endpoint() {
        let server = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let address = server.local_addr().unwrap();
        let responder = tokio::spawn(async move {
            let mut request = [0_u8; 64];
            let (length, peer) = server.recv_from(&mut request).await.unwrap();
            assert_eq!(length, 33);
            assert_eq!(request[0], 0x01);
            let motd = "MCPE;Crossplay Test;900;1.21;2;20;id;world;Survival;1;19132;19133";
            let mut response = vec![0x1c];
            response.extend_from_slice(&request[1..9]);
            response.extend_from_slice(&123_i64.to_be_bytes());
            response.extend_from_slice(&RAKNET_MAGIC);
            response.extend_from_slice(&(motd.len() as u16).to_be_bytes());
            response.extend_from_slice(motd.as_bytes());
            server.send_to(&response, peer).await.unwrap();
        });
        let config = CrossplayConfig {
            enabled: true,
            bedrock_listen: address,
            ..CrossplayConfig::default()
        };

        let status = crossplay_status(&config).await;

        assert!(status.online);
        assert!(status.motd.as_deref().unwrap().contains("Crossplay Test"));
        assert!(status.error.is_none());
        responder.await.unwrap();
    }
}
