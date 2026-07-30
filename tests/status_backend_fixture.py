#!/usr/bin/env python3
"""用于主动健康检查验收的最小 Minecraft Status 或伪 HTTP 后端。"""

import argparse
import json
import socket
import struct


def encode_varint(value: int) -> bytes:
    output = bytearray()
    while True:
        byte = value & 0x7F
        value >>= 7
        output.append(byte | (0x80 if value else 0))
        if not value:
            return bytes(output)


def read_varint(connection: socket.socket) -> int:
    value = 0
    for shift in range(0, 35, 7):
        byte = connection.recv(1)
        if not byte:
            raise EOFError
        value |= (byte[0] & 0x7F) << shift
        if not byte[0] & 0x80:
            return value
    raise ValueError("VarInt 过长")


def read_exact(connection: socket.socket, length: int) -> bytes:
    output = bytearray()
    while len(output) < length:
        chunk = connection.recv(length - len(output))
        if not chunk:
            raise EOFError
        output.extend(chunk)
    return bytes(output)


def read_packet(connection: socket.socket) -> bytes:
    return read_exact(connection, read_varint(connection))


def packet(packet_id: int, payload: bytes = b"") -> bytes:
    body = encode_varint(packet_id) + payload
    return encode_varint(len(body)) + body


def minecraft_string(value: str) -> bytes:
    encoded = value.encode()
    return encode_varint(len(encoded)) + encoded


def serve_valid(connection: socket.socket) -> None:
    read_packet(connection)
    read_packet(connection)
    response = json.dumps(
        {
            "version": {"name": "fixture", "protocol": 769},
            "players": {"max": 20, "online": 0},
            "description": {"text": "health fixture ready"},
            "forgeData": {"mods": []},
        },
        separators=(",", ":"),
    )
    connection.sendall(packet(0, minecraft_string(response)))
    ping = read_packet(connection)
    connection.sendall(encode_varint(len(ping)) + ping)


def serve_http(connection: socket.socket) -> None:
    connection.recv(4096)
    connection.sendall(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=["minecraft", "http"])
    parser.add_argument("port", type=int)
    args = parser.parse_args()
    handler = serve_valid if args.mode == "minecraft" else serve_http
    with socket.create_server(("127.0.0.1", args.port), reuse_port=True) as listener:
        while True:
            connection, _ = listener.accept()
            with connection:
                connection.settimeout(2)
                try:
                    handler(connection)
                except (EOFError, OSError, ValueError):
                    pass


if __name__ == "__main__":
    main()
