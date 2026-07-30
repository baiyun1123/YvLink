#!/usr/bin/env python3
"""顺序运行真实 Fabric/Forge/NeoForge 服务端并验收 mc-proxy。

本脚本不会接受 Minecraft EULA；每个实例的 eula.txt 必须事先由用户明确设为 true。
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import socket
import struct
import subprocess
import tempfile
import time
import urllib.request
import uuid
from dataclasses import dataclass
from pathlib import Path
from typing import Any


PROTOCOL = 767  # Minecraft Java 1.21 / 1.21.1
ADMIN_TOKEN = "modded-matrix-isolated-token-0123456789"


@dataclass(frozen=True)
class Loader:
    name: str
    root: Path
    backend_port: int
    marker: str

    def command(self) -> list[str]:
        if self.name == "fabric":
            return [
                "java",
                "-Xms256M",
                "-Xmx512M",
                "-jar",
                "fabric-server-launch.jar",
                "nogui",
            ]
        return ["./run.sh", "--nogui"]


def encode_varint(value: int) -> bytes:
    value &= 0xFFFFFFFF
    output = bytearray()
    while True:
        byte = value & 0x7F
        value >>= 7
        output.append(byte | (0x80 if value else 0))
        if not value:
            return bytes(output)


def read_varint(stream) -> int:
    value = 0
    for shift in range(0, 35, 7):
        byte = stream.read(1)
        if not byte:
            raise EOFError("连接在读取 VarInt 时关闭")
        value |= (byte[0] & 0x7F) << shift
        if not byte[0] & 0x80:
            return value
    raise ValueError("VarInt 过长")


def minecraft_string(value: str) -> bytes:
    encoded = value.encode()
    return encode_varint(len(encoded)) + encoded


def packet(packet_id: int, payload: bytes = b"") -> bytes:
    body = encode_varint(packet_id) + payload
    return encode_varint(len(body)) + body


def handshake(host: str, port: int, next_state: int) -> bytes:
    payload = (
        encode_varint(PROTOCOL)
        + minecraft_string(host)
        + struct.pack(">H", port)
        + encode_varint(next_state)
    )
    return packet(0, payload)


def read_packet(stream) -> tuple[int, bytes]:
    length = read_varint(stream)
    body = stream.read(length)
    if len(body) != length:
        raise EOFError("Minecraft 包未完整返回")
    view = memoryview(body)
    packet_id, consumed = decode_varint(view)
    return packet_id, bytes(view[consumed:])


def decode_varint(data: memoryview) -> tuple[int, int]:
    value = 0
    for index, byte in enumerate(data[:5]):
        value |= (byte & 0x7F) << (index * 7)
        if not byte & 0x80:
            return value, index + 1
    raise ValueError("VarInt 过长")


def decode_string(data: bytes) -> str:
    view = memoryview(data)
    length, consumed = decode_varint(view)
    end = consumed + length
    if end > len(view):
        raise ValueError("Minecraft 字符串越界")
    return bytes(view[consumed:end]).decode()


def query_status(host: str, port: int, virtual_host: str) -> dict[str, Any]:
    with socket.create_connection((host, port), timeout=8) as connection:
        connection.settimeout(8)
        stream = connection.makefile("rb")
        connection.sendall(handshake(virtual_host, port, 1))
        connection.sendall(packet(0))
        packet_id, payload = read_packet(stream)
        if packet_id != 0:
            raise ValueError(f"预期 Status Response 0，实际 {packet_id}")
        response = json.loads(decode_string(payload))
        ping = struct.pack(">q", 0x4D41545249580001)
        connection.sendall(packet(1, ping))
        pong_id, pong = read_packet(stream)
        if pong_id != 1 or pong != ping:
            raise ValueError("Ping/Pong 校验失败")
        return response


def offline_uuid_bytes(username: str) -> bytes:
    digest = hashlib.md5(f"OfflinePlayer:{username}".encode()).digest()
    return uuid.UUID(bytes=digest, version=3).bytes


def packet_summary(packet_id: int, payload: bytes) -> dict[str, Any]:
    return {
        "packet_id": packet_id,
        "payload_length": len(payload),
        "payload_sha256": hashlib.sha256(payload).hexdigest(),
    }


def peek_login_and_configuration(
    host: str, port: int, virtual_host: str, username: str
) -> dict[str, Any]:
    with socket.create_connection((host, port), timeout=8) as connection:
        connection.settimeout(8)
        stream = connection.makefile("rb")
        connection.sendall(handshake(virtual_host, port, 2))
        login_start = minecraft_string(username) + offline_uuid_bytes(username)
        connection.sendall(packet(0, login_start))
        login_packet_id, login_payload = read_packet(stream)
        if login_packet_id != 2:
            raise ValueError(
                f"预期 Login Success 2，实际 {login_packet_id}；"
                "测试实例必须关闭网络压缩且不要求额外 Login 协商"
            )
        # 1.20.2+ 客户端确认 Login Success 后进入 Configuration 状态。
        connection.sendall(packet(3))
        configuration_packet_id, configuration_payload = read_packet(stream)
        return {
            "login_success": packet_summary(login_packet_id, login_payload),
            "first_configuration": packet_summary(
                configuration_packet_id, configuration_payload
            ),
        }


def require_eula(loader: Loader) -> None:
    path = loader.root / "eula.txt"
    if not path.exists() or not re.search(
        r"(?m)^\s*eula\s*=\s*true\s*$", path.read_text(encoding="utf-8")
    ):
        raise RuntimeError(
            f"{loader.name}: {path} 尚未由用户设为 eula=true；脚本不会代替用户接受 EULA"
        )


def write_server_properties(loader: Loader) -> None:
    properties = {
        "server-ip": "127.0.0.1",
        "server-port": str(loader.backend_port),
        "online-mode": "false",
        "enforce-secure-profile": "false",
        "motd": f"MC Relay real matrix - {loader.name}",
        "max-players": "8",
        "view-distance": "2",
        "simulation-distance": "2",
        "sync-chunk-writes": "false",
        "generate-structures": "false",
        "level-type": "minecraft:flat",
        "level-name": "world-matrix",
        "enable-query": "false",
        "enable-rcon": "false",
        "network-compression-threshold": "-1",
    }
    source = "\n".join(f"{key}={value}" for key, value in properties.items()) + "\n"
    (loader.root / "server.properties").write_text(source, encoding="utf-8")


def wait_for_port(port: int, process: subprocess.Popen[str], timeout_seconds: int) -> None:
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError(f"进程提前退出，退出码 {process.returncode}")
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=1):
                return
        except OSError:
            time.sleep(1)
    raise TimeoutError(f"等待 127.0.0.1:{port} 启动超时")


def wait_for_minecraft_status(
    port: int,
    virtual_host: str,
    process: subprocess.Popen[str],
    timeout_seconds: int,
) -> dict[str, Any]:
    """等待服务端真正完成 Minecraft 协议初始化，而不只等待 TCP 监听。"""
    deadline = time.monotonic() + timeout_seconds
    last_error: Exception | None = None
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError(f"进程提前退出，退出码 {process.returncode}")
        try:
            return query_status("127.0.0.1", port, virtual_host)
        except (
            ConnectionError,
            EOFError,
            OSError,
            TimeoutError,
            ValueError,
            json.JSONDecodeError,
        ) as error:
            last_error = error
            time.sleep(1)
    detail = f"，最后错误: {last_error}" if last_error else ""
    raise TimeoutError(f"等待 127.0.0.1:{port} Minecraft 状态协议就绪超时{detail}")


def api_status(admin_port: int) -> dict[str, Any]:
    request = urllib.request.Request(
        f"http://127.0.0.1:{admin_port}/api/v1/status",
        headers={"Authorization": f"Bearer {ADMIN_TOKEN}"},
    )
    with urllib.request.urlopen(request, timeout=5) as response:
        return json.load(response)["data"]


def wait_for_health(
    admin_port: int, expected: str, timeout_seconds: int
) -> dict[str, Any]:
    deadline = time.monotonic() + timeout_seconds
    last: dict[str, Any] | None = None
    while time.monotonic() < deadline:
        try:
            status = api_status(admin_port)
            managed = next(
                rule for rule in status["rules"] if rule["id"] == "managed"
            )
            last = managed["backend_health"][0]
            if last["health"] == expected:
                return last
        except (OSError, KeyError, StopIteration, urllib.error.URLError):
            pass
        time.sleep(1)
    raise TimeoutError(f"等待健康状态 {expected} 超时，最后状态: {last}")


def proxy_config(
    loader: Loader, proxy_port: int, admin_port: int, config_path: Path
) -> None:
    transparent_host = f"transparent.{loader.name}.matrix.local"
    managed_host = f"managed.{loader.name}.matrix.local"
    source = f"""[admin]
listen = "127.0.0.1:{admin_port}"

[settings]
listen = "127.0.0.1:{proxy_port}"
proxy_enabled = true
max_connections = 128
connect_timeout_ms = 3000
handshake_timeout_ms = 5000
shutdown_grace_secs = 5
copy_buffer_bytes = 32768
socket_buffer_bytes = 0
listen_backlog = 128
tcp_nodelay = true
reuse_port = false
stats_interval_secs = 60

[[rules]]
id = "transparent"
name = "{loader.name} 透明转发"
host = "{transparent_host}"
backend = "127.0.0.1:{loader.backend_port}"
proxy_protocol = "off"
modify_virtual_host = false
enabled = true

[[rules]]
id = "managed"
name = "{loader.name} 后端状态托管"
host = "{managed_host}"
backend = "127.0.0.1:{loader.backend_port}"
proxy_protocol = "off"
modify_virtual_host = false
enabled = true

[rules.health_check]
enabled = true
mode = "minecraft-status"
interval_secs = 1
timeout_ms = 1000
unhealthy_threshold = 1
healthy_threshold = 1
minecraft_host = "{managed_host}"
minecraft_protocol = {PROTOCOL}

[rules.status]
mode = "backend"
cache_ttl_secs = 0
"""
    config_path.write_text(source, encoding="utf-8")


def stop_process(process: subprocess.Popen[str], graceful: bool) -> None:
    if process.poll() is not None:
        return
    if graceful and process.stdin:
        try:
            process.stdin.write("stop\n")
            process.stdin.flush()
            process.wait(timeout=30)
            return
        except (BrokenPipeError, subprocess.TimeoutExpired):
            pass
    process.terminate()
    try:
        process.wait(timeout=10)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=10)


def run_loader(
    loader: Loader,
    proxy_binary: Path,
    proxy_port: int,
    admin_port: int,
    report_dir: Path,
) -> dict[str, Any]:
    require_eula(loader)
    write_server_properties(loader)
    report_dir.mkdir(parents=True, exist_ok=True)
    server_log_path = report_dir / f"{loader.name}-server.log"
    proxy_log_path = report_dir / f"{loader.name}-proxy.log"
    report: dict[str, Any] = {
        "loader": loader.name,
        "backend": f"127.0.0.1:{loader.backend_port}",
        "protocol": PROTOCOL,
        "passed": False,
    }
    server_process: subprocess.Popen[str] | None = None
    proxy_process: subprocess.Popen[str] | None = None
    with tempfile.TemporaryDirectory(prefix=f"mc-proxy-{loader.name}-") as temporary:
        config_path = Path(temporary) / "config.toml"
        proxy_config(loader, proxy_port, admin_port, config_path)
        server_log = server_log_path.open("w", encoding="utf-8")
        proxy_log = proxy_log_path.open("w", encoding="utf-8")
        try:
            print(f"[{loader.name}] 启动真实服务端…", flush=True)
            environment = os.environ.copy()
            if loader.name != "fabric":
                environment["JAVA_TOOL_OPTIONS"] = "-Xms256M -Xmx512M"
            server_process = subprocess.Popen(
                loader.command(),
                cwd=loader.root,
                env=environment,
                stdin=subprocess.PIPE,
                stdout=server_log,
                stderr=subprocess.STDOUT,
                text=True,
            )
            transparent_host = f"transparent.{loader.name}.matrix.local"
            direct_status = wait_for_minecraft_status(
                loader.backend_port,
                transparent_host,
                server_process,
                240,
            )
            print(f"[{loader.name}] 后端协议已就绪，启动隔离代理…", flush=True)
            proxy_environment = os.environ.copy()
            proxy_environment["MC_PROXY_ADMIN_TOKEN"] = ADMIN_TOKEN
            proxy_process = subprocess.Popen(
                [str(proxy_binary), "--config", str(config_path)],
                env=proxy_environment,
                stdout=proxy_log,
                stderr=subprocess.STDOUT,
                text=True,
            )
            wait_for_port(proxy_port, proxy_process, 20)

            managed_host = f"managed.{loader.name}.matrix.local"
            transparent_status = query_status(
                "127.0.0.1", proxy_port, transparent_host
            )
            managed_status = query_status("127.0.0.1", proxy_port, managed_host)
            report["status"] = {
                "direct_equals_transparent": direct_status == transparent_status,
                "direct_equals_managed": direct_status == managed_status,
                "direct": direct_status,
                "transparent": transparent_status,
                "managed": managed_status,
            }
            if direct_status != transparent_status or direct_status != managed_status:
                raise AssertionError("直连、透明转发和后端托管 Status JSON 不一致")

            login_host = transparent_host + loader.marker
            username = f"Matrix{loader.name.title()}"[:16]
            direct_login = peek_login_and_configuration(
                "127.0.0.1", loader.backend_port, login_host, username
            )
            proxied_login = peek_login_and_configuration(
                "127.0.0.1", proxy_port, login_host, username
            )
            report["login_and_configuration"] = {
                "direct": direct_login,
                "proxied": proxied_login,
                "equal": direct_login == proxied_login,
            }
            if direct_login != proxied_login:
                raise AssertionError(
                    "直连与代理 Login Success / Configuration 首包不一致"
                )

            report["health_before_stop"] = wait_for_health(admin_port, "healthy", 15)
            stop_process(server_process, graceful=True)
            report["health_after_stop"] = wait_for_health(
                admin_port, "unhealthy", 15
            )
            report["passed"] = True
            print(
                f"[{loader.name}] Status、Login Success、Configuration 首包"
                "与健康切换通过",
                flush=True,
            )
            return report
        finally:
            if server_process:
                stop_process(server_process, graceful=True)
            if proxy_process:
                stop_process(proxy_process, graceful=False)
            server_log.close()
            proxy_log.close()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--loader", choices=["fabric", "forge", "neoforge", "all"], default="all"
    )
    parser.add_argument(
        "--matrix-root", type=Path, default=Path("/opt/mc-proxy-matrix")
    )
    parser.add_argument(
        "--proxy-binary", type=Path, default=Path("/opt/mc-proxy/mc-proxy")
    )
    parser.add_argument(
        "--report-dir",
        type=Path,
        default=Path("/opt/mc-proxy-matrix/results"),
    )
    args = parser.parse_args()
    loaders = [
        Loader(
            "fabric",
            args.matrix_root / "fabric-1.21.1",
            26701,
            "",
        ),
        Loader(
            "forge",
            args.matrix_root / "forge-1.21.1",
            26702,
            "\0FORGE",
        ),
        Loader(
            "neoforge",
            args.matrix_root / "neoforge-1.21.1",
            26703,
            "\0FORGE",
        ),
    ]
    selected = loaders if args.loader == "all" else [
        loader for loader in loaders if loader.name == args.loader
    ]
    results = []
    for index, loader in enumerate(selected):
        result = run_loader(
            loader,
            args.proxy_binary,
            proxy_port=26610 + index,
            admin_port=28110 + index,
            report_dir=args.report_dir,
        )
        results.append(result)
        (args.report_dir / f"{loader.name}.json").write_text(
            json.dumps(result, ensure_ascii=False, indent=2) + "\n",
            encoding="utf-8",
        )
    summary = {
        "generated_at": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        "passed": all(result["passed"] for result in results),
        "results": results,
    }
    args.report_dir.mkdir(parents=True, exist_ok=True)
    (args.report_dir / "summary.json").write_text(
        json.dumps(summary, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )
    print(json.dumps(summary, ensure_ascii=False, indent=2), flush=True)


if __name__ == "__main__":
    main()
