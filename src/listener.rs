use std::{io, net::SocketAddr};

use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::TcpListener;

use crate::ForwardConfig;

pub fn create_listener(config: &ForwardConfig) -> io::Result<TcpListener> {
    let domain = if config.listen.is_ipv6() {
        Domain::IPV6
    } else {
        Domain::IPV4
    };
    let socket = Socket::new(domain, Type::STREAM, Some(Protocol::TCP))?;

    socket.set_reuse_address(true)?;
    set_reuse_port(&socket, config.reuse_port)?;

    if config.socket_buffer_bytes > 0 {
        socket.set_recv_buffer_size(config.socket_buffer_bytes)?;
        socket.set_send_buffer_size(config.socket_buffer_bytes)?;
    }

    if config.listen.is_ipv6() {
        socket.set_only_v6(false)?;
    }

    socket.set_nonblocking(true)?;
    socket.bind(&config.listen.into())?;
    socket.listen(config.listen_backlog)?;

    let listener: std::net::TcpListener = socket.into();
    TcpListener::from_std(listener)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn set_reuse_port(socket: &Socket, enabled: bool) -> io::Result<()> {
    socket.set_reuse_port(enabled)
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn set_reuse_port(_socket: &Socket, enabled: bool) -> io::Result<()> {
    if enabled {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "SO_REUSEPORT is unsupported on this platform",
        ));
    }
    Ok(())
}

pub fn local_addr(listener: &TcpListener) -> io::Result<SocketAddr> {
    listener.local_addr()
}
