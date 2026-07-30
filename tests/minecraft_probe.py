#!/usr/bin/env python3
"""无第三方依赖的 Minecraft Java Status 与 Login 协议验收探针。"""

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


def packet(packet_id: int, payload: bytes = b"") -> bytes:
    body = encode_varint(packet_id) + payload
    return encode_varint(len(body)) + body


def string(value: str) -> bytes:
    encoded = value.encode()
    return encode_varint(len(encoded)) + encoded


def handshake(virtual_host: str, port: int, next_state: int, protocol: int) -> bytes:
    payload = (
        encode_varint(protocol)
        + string(virtual_host)
        + struct.pack(">H", port)
        + encode_varint(next_state)
    )
    return packet(0, payload)


def read_packet(stream) -> tuple[int, bytes]:
    length = read_varint(stream)
    body = stream.read(length)
    if len(body) != length:
        raise EOFError("Minecraft 包未完整返回")
    cursor = memoryview(body)
    packet_id, consumed = decode_varint(cursor)
    return packet_id, bytes(cursor[consumed:])


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
    return bytes(view[consumed : consumed + length]).decode()


def status(args) -> None:
    with socket.create_connection((args.host, args.port), timeout=args.timeout) as sock:
        stream = sock.makefile("rb")
        sock.sendall(handshake(args.virtual_host, args.port, 1, args.protocol))
        sock.sendall(packet(0))
        packet_id, payload = read_packet(stream)
        if packet_id != 0:
            raise ValueError(f"预期 Status Response 0，实际 {packet_id}")
        response = json.loads(decode_string(payload))
        ping = struct.pack(">q", 0x12345678)
        sock.sendall(packet(1, ping))
        pong_id, pong = read_packet(stream)
        if pong_id != 1 or pong != ping:
            raise ValueError("Ping/Pong 校验失败")
        print(json.dumps(response, ensure_ascii=False))


def login(args) -> None:
    with socket.create_connection((args.host, args.port), timeout=args.timeout) as sock:
        stream = sock.makefile("rb")
        sock.sendall(handshake(args.virtual_host, args.port, 2, args.protocol))
        sock.sendall(packet(0, string(args.username)))
        packet_id, payload = read_packet(stream)
        if packet_id != 0:
            raise ValueError(f"预期 Login Disconnect 0，实际 {packet_id}")
        print(decode_string(payload))


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=["status", "login"])
    parser.add_argument("host")
    parser.add_argument("port", type=int)
    parser.add_argument("virtual_host")
    parser.add_argument("--protocol", type=int, default=767)
    parser.add_argument("--username", default="ProbePlayer")
    parser.add_argument("--timeout", type=float, default=5)
    args = parser.parse_args()
    (status if args.mode == "status" else login)(args)


if __name__ == "__main__":
    main()
