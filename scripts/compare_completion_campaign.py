#!/usr/bin/env python3
import argparse
import json
from pathlib import Path


def load(path: Path):
    return json.loads(path.read_text())


def main():
    p = argparse.ArgumentParser(description="Compare completion-campaign summaries")
    p.add_argument("--before", required=True, help="Baseline completion summary json")
    p.add_argument("--after", required=True, help="Completion-first summary json")
    p.add_argument("--out-md", default="results/completion-campaign-compare.md")
    p.add_argument("--out-json", default="results/completion-campaign-compare.json")
    args = p.parse_args()

    b = load(Path(args.before))
    a = load(Path(args.after))

    def delta(before, after):
        d = after - before
        pct = (d / before * 100.0) if before else 0.0
        return d, pct

    p50_d, p50_pct = delta(float(b.get("p50_completion_ms", 0.0)), float(a.get("p50_completion_ms", 0.0)))
    p95_d, p95_pct = delta(float(b.get("p95_completion_ms", 0.0)), float(a.get("p95_completion_ms", 0.0)))
    p99_d, p99_pct = delta(float(b.get("p99_completion_ms", 0.0)), float(a.get("p99_completion_ms", 0.0)))
    strag_d, strag_pct = delta(float(b.get("straggler_count", 0.0)), float(a.get("straggler_count", 0.0)))
    canc_d, canc_pct = delta(
        float(b.get("canceled_redundant_sends", 0.0)),
        float(a.get("canceled_redundant_sends", 0.0)),
    )

    out = {
        "before": b,
        "after": a,
        "delta": {
            "p50_completion_ms": {"abs": p50_d, "pct": p50_pct},
            "p95_completion_ms": {"abs": p95_d, "pct": p95_pct},
            "p99_completion_ms": {"abs": p99_d, "pct": p99_pct},
            "straggler_count": {"abs": strag_d, "pct": strag_pct},
            "canceled_redundant_sends": {"abs": canc_d, "pct": canc_pct},
        },
    }

    out_json = Path(args.out_json)
    out_json.parent.mkdir(parents=True, exist_ok=True)
    out_json.write_text(json.dumps(out, indent=2) + "\n")

    lines = [
        "# Completion Campaign Comparison",
        "",
        f"- before completion_first: {b.get('completion_first')}",
        f"- after completion_first: {a.get('completion_first')}",
        "",
        f"- p50 completion (ms): {b.get('p50_completion_ms'):.3f} -> {a.get('p50_completion_ms'):.3f} ({p50_pct:+.2f}%)",
        f"- p95 completion (ms): {b.get('p95_completion_ms'):.3f} -> {a.get('p95_completion_ms'):.3f} ({p95_pct:+.2f}%)",
        f"- p99 completion (ms): {b.get('p99_completion_ms'):.3f} -> {a.get('p99_completion_ms'):.3f} ({p99_pct:+.2f}%)",
        f"- straggler count: {b.get('straggler_count')} -> {a.get('straggler_count')} ({strag_pct:+.2f}%)",
        f"- canceled redundant sends: {b.get('canceled_redundant_sends')} -> {a.get('canceled_redundant_sends')} ({canc_pct:+.2f}%)",
    ]
    out_md = Path(args.out_md)
    out_md.parent.mkdir(parents=True, exist_ok=True)
    out_md.write_text("\n".join(lines) + "\n")


if __name__ == "__main__":
    main()
