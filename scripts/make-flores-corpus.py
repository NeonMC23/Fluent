#!/usr/bin/env python3
"""Build a small parallel FLORES-200 devtest corpus.

FLORES is CC-BY-SA-4.0. Download the NLLB FLORES-200 archive separately:

    curl -L -o /tmp/flores200_dataset.tar.gz \
      https://dl.fbaipublicfiles.com/nllb/flores200_dataset.tar.gz
    tar -xzf /tmp/flores200_dataset.tar.gz -C /tmp/flores

Then run:

    python3 scripts/make-flores-corpus.py \
      --flores-dir /tmp/flores/flores200_dataset/devtest \
      --sentences 5 --output benchmarks/flores-5x10.json
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path

PAIRS = [
    ("eng_Latn", "fra_Latn"),
    ("fra_Latn", "eng_Latn"),
    ("eng_Latn", "deu_Latn"),
    ("deu_Latn", "eng_Latn"),
    ("eng_Latn", "spa_Latn"),
    ("spa_Latn", "eng_Latn"),
    ("eng_Latn", "jpn_Jpan"),
    ("jpn_Jpan", "eng_Latn"),
    ("eng_Latn", "arb_Arab"),
    ("arb_Arab", "eng_Latn"),
    ("eng_Latn", "swh_Latn"),
    ("swh_Latn", "eng_Latn"),
]


def read_lines(path: Path) -> list[str]:
    lines = []
    for raw in path.read_text(encoding="utf-8").splitlines():
        text = raw.strip()
        if text.startswith('"'):
            text = text[1:]
        if text.endswith('"'):
            text = text[:-1]
        lines.append(text)
    return lines


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--flores-dir", required=True, type=Path)
    parser.add_argument("--sentences", type=int, default=5)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()

    cache: dict[str, list[str]] = {}

    def lines(code: str) -> list[str]:
        if code not in cache:
            path = args.flores_dir / f"{code}.devtest"
            if not path.is_file():
                raise SystemExit(f"missing FLORES file: {path}")
            cache[code] = read_lines(path)
        return cache[code]

    cases = []
    for source_lang, target_lang in PAIRS:
        source = lines(source_lang)
        target = lines(target_lang)
        for index in range(args.sentences):
            cases.append(
                {
                    "id": f"flores-{source_lang}-{target_lang}-{index + 1}",
                    "category": "flores-devtest",
                    "source_lang": source_lang,
                    "target_lang": target_lang,
                    "source": source[index],
                    "reference": target[index],
                }
            )

    document = {
        "name": "FLORES-200 devtest sample",
        "license": "CC-BY-SA-4.0",
        "source": "https://dl.fbaipublicfiles.com/nllb/flores200_dataset.tar.gz",
        "notes": (
            f"First {args.sentences} devtest sentences per pair. "
            "The archive is not committed; this JSON is a small attribution "
            "sample for reproducible evaluation."
        ),
        "cases": cases,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(document, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(f"wrote {args.output} ({len(cases)} cases)")


if __name__ == "__main__":
    main()
