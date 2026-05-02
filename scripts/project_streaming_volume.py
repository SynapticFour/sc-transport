#!/usr/bin/env python3
import argparse
import json
import math
from pathlib import Path


def main():
    p = argparse.ArgumentParser(description="Project 50/100 GiB streaming durations from matrix summary")
    p.add_argument("--summary", required=True, help="streaming-matrix-summary json path")
    p.add_argument("--targets-gib", default="50,100", help="CSV target volumes in GiB")
    p.add_argument("--out-json", default="results/streaming-volume-projection.json")
    p.add_argument("--out-md", default="results/streaming-volume-projection.md")
    args = p.parse_args()

    summary = json.loads(Path(args.summary).read_text())
    targets = [float(x.strip()) for x in args.targets_gib.split(",") if x.strip()]

    rows = []
    for s in summary.get("summaries", []):
        eps = float(s.get("events_per_s_p50", 0.0))
        payload_bytes = int(s.get("payload_bytes", 0))
        if eps <= 0 or payload_bytes <= 0:
            continue
        bytes_per_s = eps * payload_bytes
        gib_per_s = bytes_per_s / (1024 ** 3)
        for target in targets:
            secs = (target * (1024 ** 3)) / bytes_per_s
            rows.append(
                {
                    "transport": s["transport"],
                    "quality": s["quality"],
                    "payload": s["payload"],
                    "payload_bytes": payload_bytes,
                    "events_per_s_p50": eps,
                    "projected_gib": target,
                    "projected_seconds": secs,
                    "projected_minutes": secs / 60.0,
                    "projected_hours": secs / 3600.0,
                    "throughput_gib_per_s": gib_per_s,
                    "fallback_total": int(s.get("fallback_total", 0)),
                    "dropped_total": int(s.get("dropped_total", 0)),
                }
            )

    out = {
        "source_repeats": summary.get("repeats"),
        "targets_gib": targets,
        "rows": rows,
    }

    out_json = Path(args.out_json)
    out_json.parent.mkdir(parents=True, exist_ok=True)
    out_json.write_text(json.dumps(out, indent=2) + "\n")

    lines = []
    lines.append("# Streaming Volume Projection")
    lines.append("")
    lines.append(f"- source repeats: {summary.get('repeats')}")
    lines.append(f"- targets GiB: {', '.join(str(t) for t in targets)}")
    lines.append("")

    # Highlight practical rows: prefer medium/large payload and low fallback where possible.
    practical = [r for r in rows if r["payload"] in {"medium", "large"}]
    practical.sort(key=lambda r: (r["transport"], r["quality"], r["projected_gib"], r["projected_minutes"]))
    for r in practical:
        lines.append(
            f"- {r['transport']}/{r['quality']}/{r['payload']} "
            f"{r['projected_gib']:.0f}GiB -> {r['projected_minutes']:.1f} min "
            f"({r['projected_hours']:.2f} h), "
            f"fallback={r['fallback_total']}, dropped={r['dropped_total']}"
        )

    out_md = Path(args.out_md)
    out_md.parent.mkdir(parents=True, exist_ok=True)
    out_md.write_text("\n".join(lines) + "\n")


if __name__ == "__main__":
    main()
