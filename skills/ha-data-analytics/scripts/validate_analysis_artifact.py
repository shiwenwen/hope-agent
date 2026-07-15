#!/usr/bin/env python3
"""Dependency-free structural validator for Hope AnalysisArtifactV1 files."""

from __future__ import annotations

import json
import pathlib
import sys
from typing import Any


SCHEMA = "hope.analysis-artifact.v1"
STATUSES = {"ready", "partial", "blocked"}


def fail(message: str) -> None:
    raise ValueError(message)


def ids(values: Any, label: str) -> set[str]:
    if not isinstance(values, list):
        fail(f"{label} must be an array")
    result: set[str] = set()
    for index, value in enumerate(values):
        if not isinstance(value, dict):
            fail(f"{label}[{index}] must be an object")
        item_id = value.get("id")
        if item_id is not None:
            if not isinstance(item_id, str) or not item_id.strip():
                fail(f"{label}[{index}].id must be a non-empty string")
            if item_id in result:
                fail(f"duplicate {label} id: {item_id}")
            result.add(item_id)
    return result


def validate(doc: Any) -> list[str]:
    if not isinstance(doc, dict):
        fail("artifact root must be an object")
    if doc.get("schemaVersion") != SCHEMA:
        fail(f"schemaVersion must be {SCHEMA!r}")
    question = doc.get("question")
    if not isinstance(question, str) or not question.strip():
        fail("question must be a non-empty string")
    if doc.get("status") not in STATUSES:
        fail("status must be ready, partial, or blocked")

    source_ids = ids(doc.get("sources", []), "sources")
    dataset_ids = ids(doc.get("datasets", []), "datasets")
    fallback_ids = ids(doc.get("staticFallbacks", []), "staticFallbacks")
    table_ids = ids(doc.get("tables", []), "tables")
    warnings: list[str] = []

    for index, source in enumerate(doc.get("sources", [])):
        digest = source.get("sha256")
        if not isinstance(digest, str) or len(digest) != 64 or any(
            char not in "0123456789abcdefABCDEF" for char in digest
        ):
            fail(f"sources[{index}].sha256 must be a 64-character hex snapshot hash")

    for index, dataset in enumerate(doc.get("datasets", [])):
        row_count = dataset.get("rowCount")
        rows = dataset.get("rows")
        if not isinstance(row_count, int) or row_count < 0:
            fail(f"datasets[{index}].rowCount must be a non-negative integer")
        if not isinstance(rows, list):
            fail(f"datasets[{index}].rows must be a bounded array")
        if len(rows) > 5000:
            fail(f"datasets[{index}] embeds more than 5000 rows")
        if len(rows) > row_count:
            fail(f"datasets[{index}] embeds more rows than rowCount")
        for source_id in dataset.get("sourceIds", []):
            if source_ids and source_id not in source_ids:
                fail(f"datasets[{index}] references unknown source {source_id!r}")

    charts = doc.get("charts", [])
    if not isinstance(charts, list):
        fail("charts must be an array")
    for index, chart in enumerate(charts):
        if not isinstance(chart, dict):
            fail(f"charts[{index}] must be an object")
        dataset = chart.get("dataset", chart.get("datasetId"))
        source = chart.get("sourceId", chart.get("source_id"))
        if not isinstance(dataset, str) or not dataset:
            fail(f"charts[{index}] is missing dataset/datasetId")
        if not isinstance(source, str) or not source:
            fail(f"charts[{index}] is missing sourceId")
        if dataset_ids and dataset not in dataset_ids:
            fail(f"charts[{index}] references unknown dataset {dataset!r}")
        if source_ids and source not in source_ids:
            fail(f"charts[{index}] references unknown source {source!r}")
        fallback = chart.get("fallbackId")
        if not fallback:
            fail(f"charts[{index}] has no fallbackId")
        elif fallback_ids or table_ids:
            if fallback not in fallback_ids | table_ids:
                fail(f"charts[{index}] references unknown fallback {fallback!r}")

    quality_checks = doc.get("dataQuality", [])
    if not isinstance(quality_checks, list):
        fail("dataQuality must be an array")
    for index, check in enumerate(quality_checks):
        if not isinstance(check, dict):
            fail(f"dataQuality[{index}] must be an object")
        for field in ("id", "check", "status", "method"):
            if not isinstance(check.get(field), str) or not check[field].strip():
                fail(f"dataQuality[{index}] is missing {field}")
        dataset = check.get("datasetId", check.get("dataset_id"))
        if not isinstance(dataset, str) or not dataset:
            fail(f"dataQuality[{index}] is missing datasetId")
        if dataset not in dataset_ids:
            fail(f"dataQuality[{index}] references unknown dataset {dataset!r}")
        if check.get("status") not in {
            "passed", "failed", "warning", "partial", "inconclusive", "not_run"
        }:
            fail(f"dataQuality[{index}] has unsupported status")
        if not isinstance(check.get("blocking"), bool):
            fail(f"dataQuality[{index}] is missing boolean blocking")

    claim_validations = doc.get("claimValidation", [])
    if not isinstance(claim_validations, list):
        fail("claimValidation must be an array")
    for index, claim in enumerate(claim_validations):
        if not isinstance(claim, dict):
            fail(f"claimValidation[{index}] must be an object")
        for field in ("claim", "metric", "denominator", "verdict", "method"):
            if not isinstance(claim.get(field), str) or not claim[field].strip():
                fail(f"claimValidation[{index}] is missing {field}")
        if claim.get("verdict") not in {
            "supported", "unsupported", "conflict", "inconclusive"
        }:
            fail(f"claimValidation[{index}] has unsupported verdict")
        claim_sources = claim.get("sourceIds", claim.get("source_ids"))
        if not isinstance(claim_sources, list) or not claim_sources:
            fail(f"claimValidation[{index}] is missing non-empty sourceIds")
        for source_id in claim_sources:
            if not isinstance(source_id, str) or source_id not in source_ids:
                fail(
                    f"claimValidation[{index}] references unknown source {source_id!r}"
                )
        confidence = claim.get("confidence")
        if confidence is not None and (
            not isinstance(confidence, (int, float))
            or isinstance(confidence, bool)
            or confidence < 0
            or confidence > 1
        ):
            fail(f"claimValidation[{index}] confidence must be between 0 and 1")

    blocking_failures = [
        check
        for check in quality_checks
        if isinstance(check, dict)
        and check.get("blocking") is True
        and check.get("status") == "failed"
    ]
    if doc.get("status") == "ready" and blocking_failures:
        fail("ready Artifact contains failed blocking data-quality checks")
    if doc.get("status") == "ready" and not doc.get("claimValidation"):
        warnings.append("ready Artifact has no claimValidation entries")
    if not source_ids:
        warnings.append("Artifact has no canonical source IDs")
    return warnings


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: validate_analysis_artifact.py path/to/artifact.json", file=sys.stderr)
        return 2
    path = pathlib.Path(sys.argv[1])
    try:
        document = json.loads(path.read_text(encoding="utf-8"))
        warnings = validate(document)
    except (OSError, json.JSONDecodeError, ValueError) as error:
        print(f"INVALID: {error}", file=sys.stderr)
        return 1
    print(f"VALID: {path}")
    for warning in warnings:
        print(f"WARNING: {warning}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
