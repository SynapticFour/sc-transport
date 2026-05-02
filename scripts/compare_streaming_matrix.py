#!/usr/bin/env python3
import argparse
import json
from pathlib import Path


def load_summary(path: Path):
    data = json.loads(path.read_text())
    idx = {}
    for s in data.get("summaries", []):
        key = (s["transport"], s["quality"], s["payload"])
        idx[key] = s
    return data, idx


def main():
    p = argparse.ArgumentParser(description="Compare streaming matrix summaries")
    p.add_argument("--before", required=True, help="Path to before summary json")
    p.add_argument("--after", required=True, help="Path to after summary json")
    p.add_argument(
        "--out-md",
        default="results/streaming-matrix-compare.md",
        help="Output markdown path",
    )
    p.add_argument(
        "--out-json",
        default="results/streaming-matrix-compare.json",
        help="Output json path",
    )
    args = p.parse_args()

    before_meta, before = load_summary(Path(args.before))
    after_meta, after = load_summary(Path(args.after))

    keys = sorted(set(before.keys()) | set(after.keys()))
    rows = []
    for key in keys:
        b = before.get(key)
        a = after.get(key)
        if not b or not a:
            continue
        b_eps = float(b.get("events_per_s_p50", 0.0))
        a_eps = float(a.get("events_per_s_p50", 0.0))
        delta = a_eps - b_eps
        delta_pct = (delta / b_eps * 100.0) if b_eps > 0 else 0.0
        rows.append(
            {
                "transport": key[0],
                "quality": key[1],
                "payload": key[2],
                "before_eps_p50": b_eps,
                "after_eps_p50": a_eps,
                "delta_eps": delta,
                "delta_pct": delta_pct,
                "before_fallback": int(b.get("fallback_total", 0)),
                "after_fallback": int(a.get("fallback_total", 0)),
                "before_dropped": int(b.get("dropped_total", 0)),
                "after_dropped": int(a.get("dropped_total", 0)),
            }
        )

    out = {
        "before_repeats": before_meta.get("repeats"),
        "after_repeats": after_meta.get("repeats"),
        "rows": rows,
    }

    out_json = Path(args.out_json)
    out_json.parent.mkdir(parents=True, exist_ok=True)
    out_json.write_text(json.dumps(out, indent=2) + "\n")

    lines = []
    lines.append("# Streaming Matrix Comparison")
    lines.append("")
    lines.append(f"- before repeats: {out['before_repeats']}")
    lines.append(f"- after repeats: {out['after_repeats']}")
    lines.append("")
    for r in rows:
        lines.append(
            f"- {r['transport']}/{r['quality']}/{r['payload']}: "
            f"p50 {r['before_eps_p50']:.2f} -> {r['after_eps_p50']:.2f} "
            f"({r['delta_pct']:+.2f}%), "
            f"fallback {r['before_fallback']} -> {r['after_fallback']}, "
            f"dropped {r['before_dropped']} -> {r['after_dropped']}"
        )
    out_md = Path(args.out_md)
    out_md.parent.mkdir(parents=True, exist_ok=True)
    out_md.write_text("\n".join(lines) + "\n")


if __name__ == "__main__":
    main()
