#!/usr/bin/env python3
"""Independently recompute the activation golden fixture using the stdlib."""

from __future__ import annotations

import csv
import json
import pathlib
import sys


def recompute(path: pathlib.Path) -> dict[str, object]:
    with path.open(newline="", encoding="utf-8") as handle:
        rows = list(csv.DictReader(handle))

    eligible = [row for row in rows if row["eligible"] == "1"]
    activated = [row for row in eligible if row["activated_7d"] == "1"]
    seen: set[str] = set()
    duplicate_ids: set[str] = set()
    by_platform: dict[str, dict[str, int]] = {}
    for row in rows:
        signup_id = row["signup_id"]
        if signup_id in seen:
            duplicate_ids.add(signup_id)
        seen.add(signup_id)
        if row["eligible"] != "1":
            continue
        bucket = by_platform.setdefault(row["platform"], {"eligible": 0, "activated": 0})
        bucket["eligible"] += 1
        bucket["activated"] += int(row["activated_7d"])

    return {
        "eligible": len(eligible),
        "activated": len(activated),
        "activationRate": len(activated) / len(eligible) if eligible else None,
        "duplicateSignupIds": sorted(duplicate_ids),
        "byPlatform": {
            platform: {
                **counts,
                "activationRate": counts["activated"] / counts["eligible"],
            }
            for platform, counts in sorted(by_platform.items())
        },
    }


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: fixture_recompute.py path/to/activation.csv", file=sys.stderr)
        return 2
    result = recompute(pathlib.Path(sys.argv[1]))
    print(json.dumps(result, indent=2, sort_keys=True))
    expected = (result["eligible"], result["activated"], result["activationRate"])
    if expected != (11, 7, 7 / 11):
        print(f"unexpected golden result: {expected}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
