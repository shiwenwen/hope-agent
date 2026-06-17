#!/usr/bin/env python3
"""End-to-end smoke test for bundled Office skills."""

from __future__ import annotations

import base64
import json
import os
import subprocess
import sys
import tempfile
import zipfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
PY = sys.executable
PNG_1X1 = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/p9sAAAAASUVORK5CYII="


def run(args: list[str], *, timeout: int = 180) -> str:
    env = {**os.environ, "PYTHONDONTWRITEBYTECODE": "1"}
    result = subprocess.run(
        args,
        cwd=ROOT,
        text=True,
        capture_output=True,
        check=True,
        timeout=timeout,
        env=env,
    )
    return result.stdout


def run_json(args: list[str], *, timeout: int = 180):
    return json.loads(run(args, timeout=timeout))


def assert_true(condition: bool, message: str, detail=None) -> None:
    if not condition:
        raise AssertionError(f"{message}: {detail!r}")


def write_json(path: Path, data: object) -> Path:
    path.write_text(json.dumps(data), encoding="utf-8")
    return path


def smoke_docx(tmp: Path) -> dict:
    tmp.mkdir(parents=True, exist_ok=True)
    image = tmp / "pixel.png"
    image.write_bytes(base64.b64decode(PNG_1X1))

    spec = write_json(
        tmp / "doc.json",
        {
            "title": "Office DOCX Smoke",
            "subtitle": "Parity verification",
            "blocks": [
                {"type": "heading", "level": 1, "text": "Summary"},
                {"type": "paragraph", "text": "Alpha contact test@example.com 415-555-1212"},
                {"type": "bullet_list", "items": ["Real list", {"text": "Commented list", "comment": "Check this"}]},
                {"type": "revision", "delete": "Old wording", "insert": "New wording"},
                {
                    "type": "image",
                    "path": str(image),
                    "alt": "One pixel verification image",
                    "caption": "Figure 1. Verification image",
                    "width_inches": 1,
                    "height_inches": 1,
                },
                {"type": "table", "headers": ["Metric", "Value"], "rows": [["ARR", "$12M"]]},
            ],
        },
    )
    docx = tmp / "doc.docx"
    run([PY, "skills/office-docx/scripts/build_docx.py", "--spec", str(spec), "--out", str(docx)])
    info = run_json([PY, "skills/office-docx/scripts/inspect_docx.py", str(docx), "--verify"])
    assert_true(info["real_list_count"] >= 2, "DOCX real lists missing", info)
    assert_true(info["comment_count"] >= 1, "DOCX comments missing", info)
    assert_true(info["tracked_insertions"] >= 1, "DOCX tracked insertions missing", info)
    assert_true(info["image_count"] == 1 and info["missing_alt_count"] == 0, "DOCX image alt failed", info)

    a11y = run_json([PY, "skills/office-docx/scripts/a11y_audit.py", str(docx)])
    assert_true(a11y["passed"], "DOCX a11y audit failed", a11y)

    sanitized = tmp / "doc-sanitized.docx"
    run([PY, "skills/office-docx/scripts/google_docs_title_sanitize.py", str(docx), "--out", str(sanitized)])
    sanitize_check = run_json([PY, "skills/office-docx/scripts/google_docs_title_sanitize.py", str(sanitized), "--check"])
    assert_true(not any(sanitize_check.values()), "DOCX Google title sanitize check failed", sanitize_check)

    comments = run_json([PY, "skills/office-docx/scripts/comments_extract.py", str(docx)])
    assert_true(len(comments) >= 1, "DOCX comments extraction failed", comments)

    replacement_spec = write_json(tmp / "replacements.json", {"replacements": [{"old": "Alpha", "new": "Beta"}]})
    redlined = tmp / "doc-redlined.docx"
    replacement = run_json(
        [
            PY,
            "skills/office-docx/scripts/add_tracked_replacements.py",
            "--input",
            str(docx),
            "--spec",
            str(replacement_spec),
            "--out",
            str(redlined),
        ]
    )
    assert_true(replacement["tracked_replacements"] >= 1, "DOCX tracked replacement failed", replacement)

    accepted = tmp / "doc-accepted.docx"
    run([PY, "skills/office-docx/scripts/accept_tracked_changes.py", str(redlined), "--mode", "accept", "--out", str(accepted)])
    stripped = tmp / "doc-no-comments.docx"
    run([PY, "skills/office-docx/scripts/comments_strip.py", str(accepted), "--out", str(stripped)])
    redacted = tmp / "doc-redacted.docx"
    run([PY, "skills/office-docx/scripts/redact_docx.py", str(stripped), str(redacted), "--emails", "--phones"])
    scrubbed = tmp / "doc-scrubbed.docx"
    run([PY, "skills/office-docx/scripts/privacy_scrub_metadata.py", str(redacted), "--out", str(scrubbed)])

    with_toc = tmp / "doc-toc.docx"
    run([PY, "skills/office-docx/scripts/insert_toc.py", str(scrubbed), "--out", str(with_toc)])
    fields = run_json([PY, "skills/office-docx/scripts/fields_report.py", str(with_toc)])
    assert_true(fields, "DOCX field report failed", fields)

    with_note = tmp / "doc-note.docx"
    run([PY, "skills/office-docx/scripts/insert_note.py", str(with_toc), "--kind", "footnote", "--text", "Source note", "--out", str(with_note)])
    watermarked = tmp / "doc-watermarked.docx"
    run([PY, "skills/office-docx/scripts/watermark_add.py", str(with_note), "--text", "DRAFT", "--out", str(watermarked)])
    watermark = run_json([PY, "skills/office-docx/scripts/watermark_audit_remove.py", str(watermarked)])
    assert_true(watermark["header_count"] >= 1, "DOCX watermark audit failed", watermark)
    watermarked_again = tmp / "doc-watermarked-again.docx"
    run([PY, "skills/office-docx/scripts/watermark_add.py", str(watermarked), "--text", "REVIEW", "--out", str(watermarked_again)])
    with zipfile.ZipFile(watermarked_again) as zf:
        header_names = sorted(name for name in zf.namelist() if name.startswith("word/header") and name.endswith(".xml"))
        header_text = "\n".join(zf.read(name).decode("utf-8") for name in header_names)
    assert_true(len(header_names) >= 2 and "DRAFT" in header_text and "REVIEW" in header_text, "DOCX watermark overwrote existing header", header_names)

    protected = tmp / "doc-protected.docx"
    run([PY, "skills/office-docx/scripts/set_protection.py", str(watermarked_again), "--mode", "readOnly", "--out", str(protected)])
    controls = write_json(tmp / "controls.json", {"controls": [{"tag": "client", "alias": "Client", "text": "Acme"}]})
    form = tmp / "doc-form.docx"
    run([PY, "skills/office-docx/scripts/content_controls.py", str(protected), "--spec", str(controls), "--out", str(form)])
    nav = write_json(
        tmp / "nav.json",
        {
            "bookmarks": [{"name": "intro", "text": "Intro bookmark"}],
            "links": [{"text": "OpenAI", "url": "https://openai.com"}, {"text": "Jump intro", "anchor": "intro"}],
        },
    )
    linked = tmp / "doc-linked.docx"
    run([PY, "skills/office-docx/scripts/internal_nav.py", str(form), "--spec", str(nav), "--out", str(linked)])
    with zipfile.ZipFile(linked) as zf:
        document_xml = zf.read("word/document.xml").decode("utf-8")
        rels_xml = zf.read("word/_rels/document.xml.rels").decode("utf-8")
    assert_true("<w:sdt>" in document_xml and "w:hyperlink" in document_xml and "hyperlink" in rels_xml, "DOCX nav/content controls failed")
    sect_idx = document_xml.rfind("<w:sectPr")
    assert_true(sect_idx < 0 or document_xml.find("<w:sdt>") < sect_idx, "DOCX content controls inserted after sectPr")
    assert_true(sect_idx < 0 or document_xml.find("w:hyperlink") < sect_idx, "DOCX links inserted after sectPr")

    table_dir = tmp / "tables"
    table_output = run([PY, "skills/office-docx/scripts/docx_table_to_csv.py", str(linked), "--out-dir", str(table_dir)])
    assert_true(bool(table_output.strip()), "DOCX table export failed", table_output)

    second_spec = write_json(tmp / "doc2.json", {"title": "Second", "blocks": [{"type": "paragraph", "text": "Merged text"}]})
    second = tmp / "doc2.docx"
    merged = tmp / "doc-merged.docx"
    run([PY, "skills/office-docx/scripts/build_docx.py", "--spec", str(second_spec), "--out", str(second)])
    run([PY, "skills/office-docx/scripts/merge_docx_append.py", "--input", str(linked), "--input", str(second), "--out", str(merged)])
    merged_info = run_json([PY, "skills/office-docx/scripts/inspect_docx.py", str(merged), "--verify"])
    assert_true("Merged text" in merged_info["text_preview"], "DOCX merge failed", merged_info)
    diff = run_json([PY, "skills/office-docx/scripts/compare_docx.py", str(docx), str(merged)])
    assert_true(diff["changed"], "DOCX compare failed", diff)

    preview = run_json([PY, "skills/office-docx/scripts/render_preview.py", str(merged)], timeout=240)
    assert_true(bool(preview.get("pdf")) and len(preview.get("pngs") or []) >= 1, "DOCX render preview failed", preview)
    return {
        "lists": info["real_list_count"],
        "comments": info["comment_count"],
        "images": info["image_count"],
        "fields": len(fields),
        "render_pngs": len(preview.get("pngs") or []),
    }


def smoke_xlsx(tmp: Path) -> dict:
    tmp.mkdir(parents=True, exist_ok=True)
    spec = write_json(
        tmp / "book.json",
        {
            "title": "Office XLSX Smoke",
            "sheets": [
                {
                    "name": "Summary",
                    "rows": [
                        ["Metric", "Value", "Status"],
                        ["A", 12, "Good"],
                        ["B", 20, "Watch"],
                        ["Median", "=MEDIAN(B2:B3)", ""],
                        ["Decision", "=IF(B2>10,ROUND(B3/3,1),ABS(-5))", ""],
                    ],
                    "tables": [{"name": "SummaryTable", "ref": "A1:C5"}],
                    "data_validations": [{"range": "C2:C10", "type": "list", "formula1": ["Good", "Watch", "Risk"]}],
                    "conditional_formats": [{"range": "B2:B10", "type": "colorScale"}],
                    "charts": [
                        {"type": "line", "title": "Trend", "categories": "$A$2:$A$3", "values": "$B$2:$B$3", "anchor": "E2"},
                        {"type": "pie", "title": "Mix", "categories": "$A$2:$A$3", "values": "$B$2:$B$3", "anchor": "E18"},
                    ],
                    "column_formats": ["text", "currency", "text"],
                    "column_widths": [18, 16, 14],
                }
            ],
        },
    )
    xlsx = tmp / "book.xlsx"
    run([PY, "skills/office-xlsx/scripts/build_xlsx.py", "--spec", str(spec), "--out", str(xlsx)])
    info = run_json([PY, "skills/office-xlsx/scripts/inspect_xlsx.py", str(xlsx), "--verify"])
    assert_true(info["chart_count"] == 2 and info["table_count"] == 1, "XLSX chart/table failed", info)
    assert_true(info["data_validation_count"] == 1 and info["conditional_format_count"] == 1, "XLSX validation/format failed", info)

    cached = tmp / "book-cached.xlsx"
    formula = run_json([PY, "skills/office-xlsx/scripts/formula_audit.py", str(xlsx), "--write-cache", str(cached), "--fail-on-unsupported"])
    assert_true(formula["evaluated_count"] >= 2 and formula["unsupported_count"] == 0, "XLSX formula audit failed", formula)

    patch = write_json(
        tmp / "patch.json",
        {"actions": [{"action": "append_rows", "sheet": "Summary", "rows": [["C", 30, "Good"]]}, {"action": "set_cell", "sheet": "Summary", "cell": "B7", "value": "=SUM(B2:B3)"}]},
    )
    patched = tmp / "book-patched.xlsx"
    run([PY, "skills/office-xlsx/scripts/patch_xlsx.py", "--input", str(cached), "--patch", str(patch), "--out", str(patched)])
    patched_info = run_json([PY, "skills/office-xlsx/scripts/inspect_xlsx.py", str(patched), "--verify"])
    assert_true(any("=SUM(B2:B3)" == item for item in patched_info["formulas"]), "XLSX patch formula missing", patched_info)

    recalc = tmp / "book-recalc.xlsx"
    recalc_result = run_json([PY, "skills/office-xlsx/scripts/recalculate_xlsx.py", str(patched), "--out", str(recalc)], timeout=240)
    assert_true(Path(recalc_result["out"]).exists(), "XLSX recalc failed", recalc_result)
    preview = run_json([PY, "skills/office-xlsx/scripts/render_preview.py", str(recalc)], timeout=240)
    assert_true(bool(preview.get("pdf")) and len(preview.get("pngs") or []) >= 1, "XLSX render failed", preview)
    return {
        "charts": info["chart_count"],
        "tables": info["table_count"],
        "formulas": formula["evaluated_count"],
        "render_pngs": len(preview.get("pngs") or []),
    }


def smoke_pptx(tmp: Path) -> dict:
    tmp.mkdir(parents=True, exist_ok=True)
    image = tmp / "pixel.png"
    image.write_bytes(base64.b64decode(PNG_1X1))
    spec = write_json(
        tmp / "deck.json",
        {
            "title": "Office PPTX Smoke",
            "slides": [
                {"type": "title", "title": "Old title"},
                {"type": "native_chart", "title": "Native mix", "chart_type": "pie", "data": [{"label": "SMB", "value": 42}, {"label": "Enterprise", "value": 58}]},
                {"type": "image", "title": "Image proof", "image": str(image), "caption": "Verified image"},
            ],
        },
    )
    deck = tmp / "deck.pptx"
    run([PY, "skills/office-pptx/scripts/build_pptx.py", "--spec", str(spec), "--out", str(deck)])
    info = run_json([PY, "skills/office-pptx/scripts/inspect_pptx.py", str(deck), "--verify"])
    assert_true(info["chart_count"] == 1 and info["graphic_frame_count"] == 1 and info["media_count"] == 1, "PPTX build failed", info)
    layout = run_json([PY, "skills/office-pptx/scripts/layout_audit.py", str(deck)])
    assert_true(layout["passed"], "PPTX layout audit failed", layout)

    patch = write_json(tmp / "patch.json", {"replace_text": [{"old": "Old title", "new": "New title"}]})
    patched = tmp / "deck-patched.pptx"
    patch_result = run_json([PY, "skills/office-pptx/scripts/patch_pptx.py", "--input", str(deck), "--patch", str(patch), "--out", str(patched)])
    assert_true(patch_result["replacements"] >= 1, "PPTX patch failed", patch_result)

    append = write_json(tmp / "append.json", {"slides": [{"type": "native_chart", "title": "Growth", "chart_type": "line", "data": [{"label": "Q1", "value": 1}, {"label": "Q2", "value": 3}]}]})
    appended = tmp / "deck-appended.pptx"
    run([PY, "skills/office-pptx/scripts/append_pptx.py", "--input", str(patched), "--spec", str(append), "--out", str(appended)])
    appended_info = run_json([PY, "skills/office-pptx/scripts/inspect_pptx.py", str(appended), "--verify"])
    assert_true(appended_info["chart_count"] == 2, "PPTX append native chart failed", appended_info)

    duplicated = tmp / "deck-duplicated.pptx"
    run([PY, "skills/office-pptx/scripts/duplicate_slide.py", "--input", str(appended), "--slide", "1", "--out", str(duplicated)])
    reordered = tmp / "deck-reordered.pptx"
    run([PY, "skills/office-pptx/scripts/deck_reorder.py", "--input", str(duplicated), "--order", "[2,1,3]", "--out", str(reordered)])
    reordered_info = run_json([PY, "skills/office-pptx/scripts/inspect_pptx.py", str(reordered), "--verify"])
    assert_true(reordered_info["slide_count"] == 3, "PPTX reorder failed", reordered_info)
    with zipfile.ZipFile(reordered) as zf:
        presentation_rels = zf.read("ppt/_rels/presentation.xml.rels").decode("utf-8")
    assert_true("slideMaster" in presentation_rels, "PPTX reorder dropped slide master relationship", presentation_rels)

    preview = run_json([PY, "skills/office-pptx/scripts/render_preview.py", str(reordered)], timeout=240)
    assert_true(bool(preview.get("pdf")) and len(preview.get("pngs") or []) >= 1, "PPTX render failed", preview)
    contact_sheet = tmp / "contact-sheet.html"
    run([PY, "skills/office-pptx/scripts/make_contact_sheet.py", "--images", *(preview.get("pngs") or []), "--out", str(contact_sheet)])
    assert_true(contact_sheet.exists(), "PPTX contact sheet failed")
    return {
        "charts": reordered_info["chart_count"],
        "slides": reordered_info["slide_count"],
        "render_pngs": len(preview.get("pngs") or []),
        "contact_sheet": contact_sheet.exists(),
    }


def main() -> None:
    with tempfile.TemporaryDirectory(prefix="office-skill-smoke-") as tmp_raw:
        tmp = Path(tmp_raw)
        report = {
            "docx": smoke_docx(tmp / "docx"),
            "xlsx": smoke_xlsx(tmp / "xlsx"),
            "pptx": smoke_pptx(tmp / "pptx"),
        }
    print(json.dumps({"passed": True, **report}, indent=2))


if __name__ == "__main__":
    main()
