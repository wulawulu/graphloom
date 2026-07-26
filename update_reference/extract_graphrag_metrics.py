#!/usr/bin/env python3
"""Extract GraphRAG model/cache metric blocks from an indexing log."""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

PATTERN = re.compile(
    r"(?P<timestamp>\d{4}-\d{2}-\d{2} [\d:.]+) - INFO - "
    r"graphrag_llm\.metrics\.log_metrics_writer - Metrics for "
    r"(?P<model>[^:]+(?:/[^:]+)?): (?P<metrics>\{.*?\n\})",
    re.DOTALL,
)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("log", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    text = args.log.read_text(encoding="utf-8")
    runs = []
    for match in PATTERN.finditer(text):
        metrics = json.loads(match.group("metrics"))
        attempted = int(metrics.get("attempted_request_count", 0))
        cached = int(metrics.get("cached_responses", 0))
        runs.append(
            {
                "timestamp": match.group("timestamp"),
                "model": match.group("model"),
                "attempted_requests": attempted,
                "cache_hits": cached,
                "cache_misses": attempted - cached,
                "cache_hit_rate": metrics.get("cache_hit_rate"),
                "raw": metrics,
            }
        )
    log_path = (
        f"<external>/{args.log.name}" if args.log.is_absolute() else str(args.log)
    )
    result = {"log": log_path, "metric_blocks": runs}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps({"output": str(args.output), "metric_blocks": len(runs)}))
    return 0 if runs else 1


if __name__ == "__main__":
    sys.exit(main())
