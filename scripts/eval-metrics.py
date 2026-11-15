#!/usr/bin/env python3
"""Summarize Fluent eval JSON with chrF++ and BLEU.

This is an analysis tool, not a runtime dependency. Install it separately:

    python3 -m venv /tmp/fluent-metrics
    /tmp/fluent-metrics/bin/pip install sacrebleu

Usage:
    /tmp/fluent-metrics/bin/python scripts/eval-metrics.py \
      --name nllb13=/tmp/nllb13.json \
      --name madlad=/tmp/madlad.json \
      --out /tmp/metrics.json
"""

from __future__ import annotations

import argparse
import json
import statistics
from collections import defaultdict
from pathlib import Path

try:
    from sacrebleu.metrics import BLEU, CHRF
except ImportError as error:
    raise SystemExit(
        "sacrebleu is required for this analysis tool; install it in a venv "
        "with `pip install sacrebleu`"
    ) from error


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--name", action="append", required=True, help="name=path.json")
    parser.add_argument("--out", type=Path)
    args = parser.parse_args()

    chrf = CHRF(word_order=2, lowercase=False)
    bleu = BLEU(tokenize="13a", lowercase=False)
    summary = {}
    for item in args.name:
        name, _, path = item.partition("=")
        if not path:
            raise SystemExit(f"expected name=path, got {item}")
        document = json.loads(Path(path).read_text(encoding="utf-8"))
        grouped = defaultdict(lambda: {"hyp": [], "ref": []})
        empty = 0
        for result in document["results"]:
            if result.get("status") != "ok" or not result.get("reference"):
                continue
            key = f"{result['source_lang']}->{result['target_lang']}"
            grouped[key]["hyp"].append(result["output"])
            grouped[key]["ref"].append(result["reference"])
            if not result["output"].strip():
                empty += 1
        pairs = {}
        for key, values in sorted(grouped.items()):
            pairs[key] = {
                "chrfpp": round(chrf.corpus_score(values["hyp"], [values["ref"]]).score, 2),
                "bleu": round(bleu.corpus_score(values["hyp"], [values["ref"]]).score, 2),
                "n": len(values["hyp"]),
            }
        summary[name] = {
            "pairs": pairs,
            "macro_chrfpp": round(statistics.mean(p["chrfpp"] for p in pairs.values()), 2)
            if pairs
            else None,
            "macro_bleu": round(statistics.mean(p["bleu"] for p in pairs.values()), 2)
            if pairs
            else None,
            "cases": sum(p["n"] for p in pairs.values()),
            "empty": empty,
            "load_ms": document.get("load_ms"),
            "peak_rss_mb": document.get("peak_rss_mb"),
        }

    text = json.dumps(summary, indent=2)
    if args.out:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(text + "\n", encoding="utf-8")
        print(f"wrote {args.out}")
    else:
        print(text)


if __name__ == "__main__":
    main()
