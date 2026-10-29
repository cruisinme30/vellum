#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 The Vellum contributors
# SPDX-License-Identifier: AGPL-3.0-only
"""Slow and idle clients against a running Vellum (red team round 4, ADR 0047).

    python3 deploy/perf/hostile/slow_clients.py http://127.0.0.1:7700 [--scale 1.0]

k6 cannot hold a raw socket open and trickle bytes, so these attacks run
here, next to hostile.js in the nightly load job:

  slow_headers  request headers sent one line every few seconds (slowloris)
  slow_body     a body trickled a byte at a time, never finishing (RUDY)
  idle_h2       HTTP/2 connections that answer pings but send no request

Each attacking connection must be closed by the server within its budget,
and a reader polling /readyz must keep getting fast answers throughout.
Exits non-zero, with a summary, when either fails. Standard library only.
"""

import argparse
import asyncio
import statistics
import sys
import time
import urllib.parse

# Budgets: the server's limits plus a margin for a loaded CI runner.
# header_read_timeout 15 s, body_timeout 120 s, idle_timeout 60 s.
BUDGET = {"slow_headers": 30.0, "slow_body": 150.0, "idle_h2": 80.0}


async def slow_headers(host, port, results):
    start = time.monotonic()
    try:
        reader, writer = await asyncio.open_connection(host, port)
        writer.write(b"GET /readyz HTTP/1.1\r\nHost: vellum\r\n")
        await writer.drain()
        for n in range(1000):
            writer.write(f"X-Slow-{n}: {'a' * 16}\r\n".encode())
            await writer.drain()
            try:
                if await asyncio.wait_for(reader.read(4096), 5) == b"":
                    break
            except asyncio.TimeoutError:
                continue
            else:
                # Any answer before the headers end is the server giving up.
                break
    except (ConnectionError, OSError):
        pass
    results["slow_headers"].append(time.monotonic() - start)


async def slow_body(host, port, results):
    start = time.monotonic()
    try:
        reader, writer = await asyncio.open_connection(host, port)
        writer.write(
            b"POST /oauth/token HTTP/1.1\r\nHost: vellum\r\n"
            b"Content-Type: application/x-www-form-urlencoded\r\n"
            b"Content-Length: 100000\r\n\r\n"
        )
        await writer.drain()
        for _ in range(100000):
            writer.write(b"a")
            await writer.drain()
            try:
                await asyncio.wait_for(reader.read(4096), 5)
                break  # an answer or a close: the server cut it off
            except asyncio.TimeoutError:
                continue
    except (ConnectionError, OSError):
        pass
    results["slow_body"].append(time.monotonic() - start)


async def idle_h2(host, port, results):
    """HTTP/2 with prior knowledge: preface, settings, then only ping answers."""
    start = time.monotonic()
    try:
        reader, writer = await asyncio.open_connection(host, port)
        writer.write(b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n" + bytes([0, 0, 0, 4, 0, 0, 0, 0, 0]))
        await writer.drain()
        while True:
            head = await reader.readexactly(9)
            length = int.from_bytes(head[:3], "big")
            payload = await reader.readexactly(length)
            kind, flags = head[3], head[4]
            if kind == 7:  # GOAWAY
                break
            if kind == 4 and not flags & 1:  # SETTINGS: acknowledge
                writer.write(bytes([0, 0, 0, 4, 1, 0, 0, 0, 0]))
            elif kind == 6 and not flags & 1:  # PING: answer like a live client
                writer.write(bytes([0, 0, 8, 6, 1, 0, 0, 0, 0]) + payload)
            await writer.drain()
    except (asyncio.IncompleteReadError, ConnectionError, OSError):
        pass
    results["idle_h2"].append(time.monotonic() - start)


async def reader_loop(host, port, stop, latencies, failures):
    """An ordinary client: one /readyz a second, on a fresh connection."""
    while not stop.is_set():
        start = time.monotonic()
        try:
            reader, writer = await asyncio.wait_for(asyncio.open_connection(host, port), 5)
            writer.write(b"GET /readyz HTTP/1.1\r\nHost: vellum\r\nConnection: close\r\n\r\n")
            await writer.drain()
            reply = await asyncio.wait_for(reader.read(64), 5)
            writer.close()
            if reply.startswith(b"HTTP/1.1 200"):
                latencies.append(time.monotonic() - start)
            else:
                failures.append(reply[:40])
        except (asyncio.TimeoutError, ConnectionError, OSError) as e:
            failures.append(repr(e))
        await asyncio.sleep(1)


async def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("base_url")
    parser.add_argument("--scale", type=float, default=1.0, help="multiplies the connection counts")
    args = parser.parse_args()
    url = urllib.parse.urlparse(args.base_url)
    host, port = url.hostname, url.port or 80
    counts = {"slow_headers": 300, "slow_body": 100, "idle_h2": 100}
    attacks = {"slow_headers": slow_headers, "slow_body": slow_body, "idle_h2": idle_h2}
    results = {name: [] for name in attacks}
    stop = asyncio.Event()
    latencies, failures = [], []
    reader_task = asyncio.create_task(reader_loop(host, port, stop, latencies, failures))
    tasks = [
        asyncio.wait_for(attacks[name](host, port, results), BUDGET[name] + 30)
        for name, n in counts.items()
        for _ in range(max(1, int(n * args.scale)))
    ]
    outcomes = await asyncio.gather(*tasks, return_exceptions=True)
    stop.set()
    await reader_task
    held_forever = sum(isinstance(o, asyncio.TimeoutError) for o in outcomes)

    ok = held_forever == 0
    for name, times in results.items():
        over = [t for t in times if t > BUDGET[name]]
        worst = max(times, default=0.0)
        print(f"{name}: {len(times)} closed, slowest after {worst:.1f}s (budget {BUDGET[name]:.0f}s), {len(over)} over budget")
        ok &= not over
    if held_forever:
        print(f"{held_forever} attacking connections were never closed")
    p95 = statistics.quantiles(latencies, n=20)[-1] if len(latencies) >= 20 else max(latencies, default=0.0)
    print(f"reader: {len(latencies)} answers, p95 {p95 * 1000:.0f} ms, {len(failures)} failures")
    ok &= not failures and p95 < 1.0
    if failures:
        print(f"reader failures: {failures[:5]}")
    print("PASS" if ok else "FAIL")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(asyncio.run(main()))
