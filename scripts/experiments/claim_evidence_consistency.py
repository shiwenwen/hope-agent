#!/usr/bin/env python3
"""FTR-005: local, synthetic-only cross-artifact evidence consistency PoC.

All fixtures are in memory. This does not read user files, invoke a model, repair
artifacts, or change the production Goal/Domain Quality gate. The small typed
rules demonstrate evidence versioning, not general natural-language validity.
"""

import copy
import hashlib
import json


def digest(record):
    encoded = json.dumps(record, sort_keys=True, separators=(",", ":"), ensure_ascii=False)
    return hashlib.sha256(encoded.encode("utf-8")).hexdigest()


def baseline_fixture():
    source = {
        "project": "project-a", "incognito": False,
        "records": {
            "north": {"scope": "shared", "amount": 10},
            "south": {"scope": "shared", "amount": 20},
            "note": {"scope": "shared", "text": "Reviewed"},
            "unrelated": {"scope": "shared", "text": "Unchanged"},
        },
    }
    other = {"project": "project-b", "incognito": False,
             "records": {"south": {"scope": "shared", "amount": 20}}}
    citations = {
        key: {"artifact": "source/table.csv", "record": key, "version": digest(row)}
        for key, row in source["records"].items()
    }
    return {
        "artifacts": {
            "source/table.csv": source,
            "other/table.csv": other,
            "summary.json": {"project": "project-a", "incognito": False},
            "report.md": {"project": "project-a", "incognito": False},
            "citations.json": {"project": "project-a", "incognito": False},
        },
        "citations": citations,
        "claims": {
            "summary-total": {"artifact": "summary.json", "project": "project-a",
                              "scope": "shared", "rule": "sum-amount", "value": 30,
                              "supports": ["north", "south"]},
            "report-total": {"artifact": "report.md", "project": "project-a",
                             "scope": "shared", "rule": "sum-amount", "value": 30,
                             "supports": ["north", "south"]},
            "report-note": {"artifact": "report.md", "project": "project-a",
                            "scope": "shared", "rule": "copy-text", "value": "Reviewed",
                            "supports": ["note"]},
            "report-unrelated": {"artifact": "report.md", "project": "project-a",
                                 "scope": "shared", "rule": "copy-text", "value": "Unchanged",
                                 "supports": ["unrelated"]},
        },
    }


def evaluate(bundle):
    """Return claim validity; exact project, record and digest are mandatory."""
    artifacts = bundle["artifacts"]
    citations = bundle["citations"]
    results = {}
    for claim_id, claim in bundle["claims"].items():
        output = artifacts.get(claim["artifact"], {})
        valid = (output.get("project") == claim["project"]
                 and output.get("incognito") is False
                 and bool(claim["supports"]))
        values = []
        for ref_id in claim["supports"]:
            citation = citations.get(ref_id, {})
            source = artifacts.get(citation.get("artifact"), {})
            record = source.get("records", {}).get(citation.get("record"))
            if (source.get("project") != claim["project"]
                    or source.get("incognito") is not False
                    or not isinstance(record, dict)
                    or record.get("scope") != claim["scope"]
                    or citation.get("version") != digest(record)):
                valid = False
                break
            values.append(record)
        if valid and claim["rule"] == "sum-amount":
            valid = (all(type(row.get("amount")) is int for row in values)
                     and sum(row["amount"] for row in values) == claim["value"])
        elif valid and claim["rule"] == "copy-text":
            valid = (len(values) == 1 and isinstance(values[0].get("text"), str)
                     and values[0]["text"] == claim["value"])
        else:
            valid = valid and claim["rule"] in ("sum-amount", "copy-text")
        results[claim_id] = valid
    return results


def completeness_only(bundle):
    """A deliberately weak single-artifact baseline: presence, not freshness."""
    return {key: bool(value["supports"])
            and all(ref in bundle["citations"] for ref in value["supports"])
            for key, value in bundle["claims"].items()}


def fixtures():
    cases = []
    numeric = baseline_fixture()
    numeric["artifacts"]["source/table.csv"]["records"]["north"]["amount"] = 11
    cases.append(("numeric-change", numeric, {"summary-total", "report-total"}))

    narrowed = baseline_fixture()
    narrowed["artifacts"]["source/table.csv"]["records"]["note"]["scope"] = "private"
    cases.append(("scope-narrowing", narrowed, {"report-note"}))

    missing = baseline_fixture()
    del missing["artifacts"]["source/table.csv"]["records"]["south"]
    # A same-name record in another project must not satisfy the old citation.
    cases.append(("citation-invalidated", missing, {"summary-total", "report-total"}))

    unrelated = baseline_fixture()
    unrelated["artifacts"]["source/table.csv"]["records"]["unrelated"]["text"] = "Updated"
    cases.append(("unrelated-edit", unrelated, {"report-unrelated"}))
    return cases


def self_test():
    original = baseline_fixture()
    assert all(evaluate(original).values())
    expected_ids = set(original["claims"])
    results = []
    for name, bundle, stale in fixtures():
        expected = {key: key not in stale for key in expected_ids}
        runs = [evaluate(copy.deepcopy(bundle)) for _ in range(3)]
        assert all(run == expected for run in runs), name
        assert all(completeness_only(bundle).values()), name
        results.append({"case": name, "staleClaims": sorted(stale),
                        "validClaims": sorted(expected_ids - stale),
                        "repetitions": len(runs)})

    # A forged pointer to a same-name record in another project cannot revive it.
    foreign = baseline_fixture()
    foreign["citations"]["south"]["artifact"] = "other/table.csv"
    assert not evaluate(foreign)["summary-total"]
    private = baseline_fixture()
    private["artifacts"]["source/table.csv"]["incognito"] = True
    assert not any(evaluate(private).values())
    return {"evidence": "synthetic-only", "cases": results,
            "crossProjectRejected": True, "incognitoRejected": True,
            "productionDataRead": False, "modelQualityValidated": False}


if __name__ == "__main__":
    print(json.dumps(self_test(), ensure_ascii=False, indent=2))
