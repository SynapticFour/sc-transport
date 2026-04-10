#!/usr/bin/env python3
import json
import sys
from pathlib import Path


def load_json(path: Path):
    if not path.exists():
        return {}
    with path.open("r", encoding="utf-8") as f:
        return json.load(f)


def main() -> int:
    if len(sys.argv) not in (3, 4):
        print("usage: compare_bench_results.py <previous.json> <current.json> [regressions_out.txt]")
        return 1

    previous = load_json(Path(sys.argv[1]))
    current = load_json(Path(sys.argv[2]))

    if not previous:
        print("No previous results; skipping regression detection.")
        return 0
    if not current:
        print("No current results; skipping regression detection.")
        return 0

    regressions = []
    for key, curr in current.items():
        prev = previous.get(key)
        if prev is None:
            continue
        try:
            prev_v = float(prev)
            curr_v = float(curr)
        except (TypeError, ValueError):
            continue
        # Throughput-like metrics: lower is worse.
        if key.endswith("_mbps") or "throughput" in key.lower() or "delivery_ratio" in key.lower():
            if curr_v < prev_v * 0.80:
                regressions.append((key, prev_v, curr_v))
            continue
        # Time/latency-like metrics: higher is worse.
        if curr_v > prev_v * 1.20:
            regressions.append((key, prev_v, curr_v))

    out_path = Path(sys.argv[3]) if len(sys.argv) == 4 else None

    if regressions:
        print("Significant benchmark changes detected:")
        for key, prev_v, curr_v in regressions:
            print(f"- {key}: {prev_v:.4f} -> {curr_v:.4f}")
        print("Create/update a tracking issue in GitHub.")
        if out_path is not None:
            with out_path.open("w", encoding="utf-8") as f:
                f.write("true\n")
    else:
        print("No significant benchmark regressions detected.")
        if out_path is not None:
            with out_path.open("w", encoding="utf-8") as f:
                f.write("false\n")

    # Never block merges for experimental jobs.
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
