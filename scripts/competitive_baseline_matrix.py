#!/usr/bin/env python3
import argparse
import json
import socket
import threading
import time
from pathlib import Path


PROFILES = [
    {"name": "excellent", "base_latency_ms": 0, "jitter_ms": 0, "loss_pct": 0.0},
    {"name": "good", "base_latency_ms": 1, "jitter_ms": 1, "loss_pct": 0.2},
    {"name": "poor", "base_latency_ms": 5, "jitter_ms": 3, "loss_pct": 3.0},
    {"name": "very-poor", "base_latency_ms": 10, "jitter_ms": 6, "loss_pct": 12.0},
]

PAYLOADS = [
    {"name": "medium", "bytes": 65536, "events": 20},
    {"name": "large", "bytes": 262144, "events": 20},
]


def should_simulate_loss(idx: int, loss_pct: float) -> bool:
    if loss_pct <= 0:
        return False
    threshold = int(round(loss_pct * 10.0))
    return ((idx * 37 + 11) % 1000) < threshold


def sleep_for(profile: dict, idx: int):
    jitter = 0
    if profile["jitter_ms"] > 0:
        jitter = (idx * 17) % (profile["jitter_ms"] + 1)
    ms = profile["base_latency_ms"] + jitter
    if ms > 0:
        time.sleep(ms / 1000.0)


def run_tcp_case(profile: dict, payload_bytes: int, events: int, repeat: int):
    payload = b"x" * payload_bytes
    received = {"bytes": 0}
    ready = threading.Event()
    done = threading.Event()
    bind = {}

    def server():
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
            s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            s.bind(("127.0.0.1", 0))
            s.listen(1)
            bind["addr"] = s.getsockname()
            ready.set()
            conn, _ = s.accept()
            with conn:
                while True:
                    buf = conn.recv(131072)
                    if not buf:
                        break
                    received["bytes"] += len(buf)
        done.set()

    t = threading.Thread(target=server, daemon=True)
    t.start()
    ready.wait(timeout=2)
    addr = bind["addr"]

    sent_events = 0
    started = time.perf_counter()
    with socket.create_connection(addr, timeout=2) as c:
        for i in range(events):
            c.sendall(payload)
            sent_events += 1
            sleep_for(profile, i)
    elapsed = max(time.perf_counter() - started, 1e-6)
    done.wait(timeout=2)
    return {
        "transport": "baseline-tcp",
        "quality": profile["name"],
        "payload": "medium" if payload_bytes == 65536 else "large",
        "payload_bytes": payload_bytes,
        "repeat": repeat,
        "events_attempted": events,
        "sent": sent_events,
        "dropped": 0,
        "fallback": 0,
        "elapsed_s": elapsed,
        "events_per_s": sent_events / elapsed,
        "recv_bytes": received["bytes"],
    }


def run_udp_case(profile: dict, payload_bytes: int, events: int, repeat: int):
    payload = b"x" * min(payload_bytes, 1200)
    received = {"pkts": 0}
    ready = threading.Event()
    stop = threading.Event()
    bind = {}

    def server():
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as s:
            s.bind(("127.0.0.1", 0))
            bind["addr"] = s.getsockname()
            ready.set()
            s.settimeout(0.2)
            while not stop.is_set():
                try:
                    data, _ = s.recvfrom(2048)
                    if data:
                        received["pkts"] += 1
                except socket.timeout:
                    continue

    t = threading.Thread(target=server, daemon=True)
    t.start()
    ready.wait(timeout=2)
    addr = bind["addr"]

    sent_events = 0
    dropped = 0
    started = time.perf_counter()
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as c:
        for i in range(events):
            if should_simulate_loss(i, profile["loss_pct"]):
                dropped += 1
            else:
                c.sendto(payload, addr)
                sent_events += 1
            sleep_for(profile, i)
    elapsed = max(time.perf_counter() - started, 1e-6)
    time.sleep(0.1)
    stop.set()
    return {
        "transport": "baseline-udp",
        "quality": profile["name"],
        "payload": "medium" if payload_bytes == 65536 else "large",
        "payload_bytes": payload_bytes,
        "repeat": repeat,
        "events_attempted": events,
        "sent": sent_events,
        "dropped": dropped,
        "fallback": 0,
        "elapsed_s": elapsed,
        "events_per_s": sent_events / elapsed,
        "recv_bytes": received["pkts"] * len(payload),
    }


def percentile(values, q):
    if not values:
        return 0.0
    values = sorted(values)
    idx = max(0, min(len(values) - 1, int(round(q * (len(values) - 1)))))
    return values[idx]


def aggregate(rows):
    grouped = {}
    for r in rows:
        key = (r["transport"], r["quality"], r["payload"], r["payload_bytes"])
        grouped.setdefault(key, []).append(r)
    out = []
    for (transport, quality, payload, payload_bytes), vals in grouped.items():
        eps = [v["events_per_s"] for v in vals]
        out.append(
            {
                "transport": transport,
                "quality": quality,
                "payload": payload,
                "payload_bytes": payload_bytes,
                "runs": len(vals),
                "events_per_s_p50": percentile(eps, 0.5),
                "events_per_s_p95": percentile(eps, 0.95),
                "dropped_total": sum(v["dropped"] for v in vals),
                "fallback_total": sum(v["fallback"] for v in vals),
            }
        )
    return out


def projection_minutes(events_per_s, payload_bytes, gib):
    if events_per_s <= 0:
        return float("inf")
    bytes_per_s = events_per_s * payload_bytes
    seconds = gib * (1024 ** 3) / bytes_per_s
    return seconds / 60.0


def main():
    ap = argparse.ArgumentParser(description="Competitive baseline matrix (streaming-only)")
    ap.add_argument("--sc-summary", required=True, help="path to streaming-matrix-summary json")
    ap.add_argument("--repeats", type=int, default=5)
    ap.add_argument("--out-json", default="results/competitive-baseline-matrix.json")
    ap.add_argument("--out-md", default="results/competitive-baseline-matrix.md")
    args = ap.parse_args()

    sc_summary = json.loads(Path(args.sc_summary).read_text())
    baseline_rows = []
    for rep in range(args.repeats):
        for prof in PROFILES:
            for pl in PAYLOADS:
                baseline_rows.append(run_tcp_case(prof, pl["bytes"], pl["events"], rep))
                baseline_rows.append(run_udp_case(prof, pl["bytes"], pl["events"], rep))

    baseline_summary = aggregate(baseline_rows)

    sc_rows = []
    for s in sc_summary.get("summaries", []):
        if s["payload"] not in {"medium", "large"}:
            continue
        sc_rows.append(
            {
                "transport": s["transport"],
                "quality": s["quality"],
                "payload": s["payload"],
                "payload_bytes": s["payload_bytes"],
                "runs": s["runs"],
                "events_per_s_p50": s["events_per_s_p50"],
                "events_per_s_p95": s["events_per_s_p95"],
                "dropped_total": s.get("dropped_total", 0),
                "fallback_total": s.get("fallback_total", 0),
            }
        )

    all_rows = sc_rows + baseline_summary
    out = {"rows": all_rows, "baseline_repeats": args.repeats}
    out_json = Path(args.out_json)
    out_json.parent.mkdir(parents=True, exist_ok=True)
    out_json.write_text(json.dumps(out, indent=2) + "\n")

    # ranking by projected 100GiB time per quality/payload
    md = []
    md.append("# Competitive Baseline Matrix")
    md.append("")
    md.append(f"- baseline repeats: {args.repeats}")
    md.append("- compared transports: sc transports + raw TCP + raw UDP baselines")
    md.append("")
    for quality in [p["name"] for p in PROFILES]:
        for payload in ["medium", "large"]:
            rows = [r for r in all_rows if r["quality"] == quality and r["payload"] == payload]
            for r in rows:
                r["proj_50_min"] = projection_minutes(r["events_per_s_p50"], r["payload_bytes"], 50)
                r["proj_100_min"] = projection_minutes(r["events_per_s_p50"], r["payload_bytes"], 100)
            rows.sort(key=lambda r: r["proj_100_min"])
            md.append(f"## {quality} / {payload}")
            for r in rows:
                md.append(
                    f"- {r['transport']}: p50={r['events_per_s_p50']:.2f} eps, "
                    f"50GiB={r['proj_50_min']:.1f} min, 100GiB={r['proj_100_min']:.1f} min, "
                    f"fallback={r['fallback_total']}, dropped={r['dropped_total']}"
                )
            md.append("")

    out_md = Path(args.out_md)
    out_md.parent.mkdir(parents=True, exist_ok=True)
    out_md.write_text("\n".join(md) + "\n")


if __name__ == "__main__":
    main()
