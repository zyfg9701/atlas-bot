#!/usr/bin/env python3
"""Minimal Bot-Relay WS closed-loop against a live Hub (stdlib only).

Used by tools/p4-smoke.sh to prove P3.5 mock-CLI non-echo on the same wire
path mobile clients use (hello → cold → subscribe → sendPrompt → turn_finished
→ transcriptTail).
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
import socket
import struct
import sys
import time
from typing import Any, Optional
from urllib.parse import urlparse


def _recv_exact(sock: socket.socket, n: int) -> bytes:
    buf = b""
    while len(buf) < n:
        chunk = sock.recv(n - len(buf))
        if not chunk:
            raise ConnectionError("socket closed")
        buf += chunk
    return buf


def _ws_connect(url: str, timeout: float = 10.0) -> socket.socket:
    u = urlparse(url)
    host = u.hostname or "127.0.0.1"
    port = u.port or (443 if u.scheme == "wss" else 80)
    path = u.path or "/"
    if u.query:
        path += "?" + u.query
    key = base64.b64encode(os.urandom(16)).decode()
    sock = socket.create_connection((host, port), timeout=timeout)
    req = (
        f"GET {path} HTTP/1.1\r\n"
        f"Host: {host}:{port}\r\n"
        "Upgrade: websocket\r\n"
        "Connection: Upgrade\r\n"
        f"Sec-WebSocket-Key: {key}\r\n"
        "Sec-WebSocket-Version: 13\r\n"
        "\r\n"
    )
    sock.sendall(req.encode())
    # Read HTTP response headers
    data = b""
    while b"\r\n\r\n" not in data:
        chunk = sock.recv(4096)
        if not chunk:
            raise ConnectionError("no WS handshake response")
        data += chunk
    header, _ = data.split(b"\r\n\r\n", 1)
    status = header.split(b"\r\n", 1)[0]
    if b"101" not in status:
        raise ConnectionError(f"WS upgrade failed: {status!r}")
    return sock


def _ws_send(sock: socket.socket, text: str) -> None:
    payload = text.encode()
    header = bytearray([0x81])  # text, FIN
    mask_bit = 0x80
    n = len(payload)
    if n < 126:
        header.append(mask_bit | n)
    elif n < 65536:
        header.append(mask_bit | 126)
        header.extend(struct.pack("!H", n))
    else:
        header.append(mask_bit | 127)
        header.extend(struct.pack("!Q", n))
    mask = os.urandom(4)
    header.extend(mask)
    masked = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
    sock.sendall(bytes(header) + masked)


def _ws_recv(sock: socket.socket) -> str:
    while True:
        b1, b2 = _recv_exact(sock, 2)
        opcode = b1 & 0x0F
        masked = (b2 & 0x80) != 0
        length = b2 & 0x7F
        if length == 126:
            length = struct.unpack("!H", _recv_exact(sock, 2))[0]
        elif length == 127:
            length = struct.unpack("!Q", _recv_exact(sock, 8))[0]
        mask = _recv_exact(sock, 4) if masked else b""
        payload = _recv_exact(sock, length)
        if masked:
            payload = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
        if opcode == 0x8:  # close
            raise ConnectionError("WS closed by peer")
        if opcode == 0x9:  # ping → pong
            # echo pong
            hdr = bytearray([0x8A, 0x80 | len(payload)])
            m = os.urandom(4)
            hdr.extend(m)
            sock.sendall(bytes(hdr) + bytes(b ^ m[i % 4] for i, b in enumerate(payload)))
            continue
        if opcode in (0x1, 0x2, 0x0):
            return payload.decode()
        # ignore other control


def _rpc(sock: socket.socket, rid: int, method: str, params: dict) -> Any:
    _ws_send(
        sock,
        json.dumps({"jsonrpc": "2.0", "id": rid, "method": method, "params": params}),
    )
    deadline = time.time() + 30
    events = []
    while time.time() < deadline:
        sock.settimeout(max(0.1, deadline - time.time()))
        try:
            raw = _ws_recv(sock)
        except socket.timeout:
            continue
        msg = json.loads(raw)
        if msg.get("method") == "bot.event":
            events.append(msg)
            continue
        if msg.get("id") == rid:
            if "error" in msg:
                raise RuntimeError(f"RPC error {method}: {msg['error']}")
            return msg.get("result"), events
    raise TimeoutError(f"timeout waiting for {method} id={rid}")


def _wait_turn_finished(sock: socket.socket, agent_id: str, timeout: float = 20.0) -> dict:
    deadline = time.time() + timeout
    while time.time() < deadline:
        sock.settimeout(max(0.1, deadline - time.time()))
        try:
            raw = _ws_recv(sock)
        except socket.timeout:
            continue
        msg = json.loads(raw)
        if msg.get("method") != "bot.event":
            continue
        params = msg.get("params") or {}
        if params.get("agentId") == agent_id and params.get("channel") == "hub:turn_finished":
            return params
    raise TimeoutError("hub:turn_finished timeout")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--url", default=os.environ.get("ATLAS_HUB_WS", "ws://127.0.0.1:7700/ws"))
    ap.add_argument("--prompt", default="hello-p4-ws-smoke")
    ap.add_argument("--agent", default="agt_1")
    args = ap.parse_args()

    sock = _ws_connect(args.url)
    _ws_send(sock, json.dumps({"protocol_version": "1.0.0", "kind": "bot_client"}))
    hello = json.loads(_ws_recv(sock))
    assert "connection_id" in hello, hello
    print("← hello_ack", hello.get("connection_id"), flush=True)

    status, _ = _rpc(sock, 1, "bot.status", {})
    print("← status", status.get("runState"), flush=True)
    roster, _ = _rpc(sock, 2, "bot.roster", {})
    agents = [a.get("agentId") for a in (roster.get("agents") or [])]
    print("← roster", agents, flush=True)
    agent = args.agent if args.agent in agents else (agents[0] if agents else args.agent)

    _rpc(sock, 3, "bot.subscribe", {"agentIds": [agent]})
    print("→ subscribed", agent, flush=True)

    send, pending_events = _rpc(
        sock,
        4,
        "bot.command",
        {
            "agentId": agent,
            "name": "sendPrompt",
            "args": {"agentId": agent, "prompt": args.prompt},
        },
    )
    preview = (send or {}).get("preview") or ""
    print("← sendPrompt preview:", preview, flush=True)

    saw_tf = any(
        (e.get("params") or {}).get("channel") == "hub:turn_finished" for e in pending_events
    )
    if not saw_tf:
        tf = _wait_turn_finished(sock, agent)
        print("← hub:turn_finished", tf.get("event"), flush=True)
    else:
        print("← hub:turn_finished (with sendPrompt)", flush=True)

    tail, _ = _rpc(
        sock,
        5,
        "bot.command",
        {
            "agentId": agent,
            "name": "getAgentTranscriptTail",
            "args": {"id": agent, "limit": 20},
        },
    )
    entries = (tail or {}).get("entries") or []
    texts = [e.get("text", "") for e in entries]
    print("← transcriptTail:", texts, flush=True)

    blob = "\n".join(texts) + "\n" + preview
    if "echo:" in blob and "atlas-mock-reply" not in blob:
        print("FAIL: stub echo only — start Hub with ATLAS_GATEWAY_URL + mock CLI", file=sys.stderr)
        return 2
    if "atlas-mock-reply" not in blob:
        print(f"FAIL: expected non-echo atlas-mock-reply in transcript/preview, got {blob!r}", file=sys.stderr)
        return 2
    if args.prompt == preview:
        print("FAIL: preview equals prompt (echo)", file=sys.stderr)
        return 2

    print(
        "SMOKE_OK p4 ws-hub gateway-mock non-echo+hello+cold+subscribe+sendPrompt+turn_finished+transcriptTail",
        flush=True,
    )
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as e:
        print(f"FAIL: {e}", file=sys.stderr)
        raise SystemExit(1)
