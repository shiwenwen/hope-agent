---
name: office-docx
description: "Use when the user asks to create, edit, inspect, polish, verify, or deliver Word `.docx` documents, Google Docs-targeted drafts, business briefs, forms, reports, tables, checklists, redraft-ready document sections, or PDF/Word source-to-DOCX transformations."
requires:
  bins: [python3]
install:
  - kind: brew
    formula: python
    bins: [python3]
    label: "Install Python 3 via Homebrew"
    os: [darwin]
allowed-tools:
  - read
  - write
  - edit
  - apply_patch
  - exec
  - ls
  - grep
  - pdf
  - web_search
  - web_fetch
  - send_attachment
---

# Office DOCX

Use the bundled scripts in this skill package to produce editable `.docx`
files. The builder writes deterministic OOXML with real Word numbering,
tables, images with alt text, comments, and tracked changes. The skill
activation metadata includes `Skill directory`; treat that as `SKILL_DIR` and
run scripts from `SKILL_DIR/scripts/`.

## Workflow

1. Decide the document archetype: brief, proposal, SOP, form, checklist,
   report, memo, or reference guide.
2. Create a JSON spec in the working directory. Use semantic blocks:
   `heading`, `paragraph`, `bullet_list`, `numbered_list`, `table`, `callout`,
   `image`, `comment`, `revision`, and `page_break`.
3. For a new document, run:

```bash
python3 "$SKILL_DIR/scripts/check_env.py"
python3 "$SKILL_DIR/scripts/build_docx.py" --spec spec.json --out output.docx
python3 "$SKILL_DIR/scripts/inspect_docx.py" --verify output.docx
python3 "$SKILL_DIR/scripts/a11y_audit.py" output.docx
```

4. To append to an existing document, use a spec containing only the new
   `blocks`, then run:

```bash
python3 "$SKILL_DIR/scripts/append_docx.py" --input existing.docx --spec append.json --out output.docx
python3 "$SKILL_DIR/scripts/inspect_docx.py" --verify output.docx
```

5. To convert a Markdown draft directly:

```bash
python3 "$SKILL_DIR/scripts/markdown_to_docx.py" --markdown draft.md --out output.docx
python3 "$SKILL_DIR/scripts/inspect_docx.py" --verify output.docx
```

6. For Google Docs-targeted output, sanitize before render/import:

```bash
python3 "$SKILL_DIR/scripts/google_docs_title_sanitize.py" output.docx --out sanitized.docx
python3 "$SKILL_DIR/scripts/google_docs_title_sanitize.py" sanitized.docx --check
```

Use `sanitized.docx` for preview rendering and native Google Docs import.

7. For redline/comment work, use `comment` blocks or `revision` blocks:

```json
{"type": "comment", "target": "This sentence needs support.", "comment": "Add source."}
{"type": "revision", "delete": "old wording", "insert": "new wording"}
```

8. For delivery cleanup, use the focused helpers:

```bash
python3 "$SKILL_DIR/scripts/comments_strip.py" commented.docx --out no-comments.docx
python3 "$SKILL_DIR/scripts/comments_extract.py" commented.docx
python3 "$SKILL_DIR/scripts/add_tracked_replacements.py" --input input.docx --spec replacements.json --out redlined.docx
python3 "$SKILL_DIR/scripts/accept_tracked_changes.py" redlined.docx --mode accept --out accepted.docx
python3 "$SKILL_DIR/scripts/redact_docx.py" input.docx redacted.docx --emails --phones
python3 "$SKILL_DIR/scripts/privacy_scrub_metadata.py" input.docx --out scrubbed.docx
python3 "$SKILL_DIR/scripts/compare_docx.py" before.docx after.docx
```

9. For advanced Word structure tasks, use the focused OOXML helpers:

```bash
python3 "$SKILL_DIR/scripts/insert_toc.py" input.docx --out with-toc.docx
python3 "$SKILL_DIR/scripts/fields_report.py" with-toc.docx
python3 "$SKILL_DIR/scripts/insert_note.py" input.docx --kind footnote --text "Source note" --out with-note.docx
python3 "$SKILL_DIR/scripts/watermark_add.py" input.docx --text "DRAFT" --out watermarked.docx
python3 "$SKILL_DIR/scripts/watermark_audit_remove.py" watermarked.docx
python3 "$SKILL_DIR/scripts/set_protection.py" input.docx --mode readOnly --out protected.docx
python3 "$SKILL_DIR/scripts/content_controls.py" input.docx --spec controls.json --out form.docx
python3 "$SKILL_DIR/scripts/internal_nav.py" input.docx --spec nav.json --out linked.docx
python3 "$SKILL_DIR/scripts/merge_docx_append.py" --input a.docx --input b.docx --out merged.docx
python3 "$SKILL_DIR/scripts/docx_table_to_csv.py" input.docx --out-dir tables
```

10. If visual QA matters and LibreOffice is available, render previews:

```bash
python3 "$SKILL_DIR/scripts/render_preview.py" output.docx
```

11. Deliver the `.docx` path or attach it with `send_attachment`.

## Spec Shape

```json
{
  "title": "Document title",
  "subtitle": "Optional subtitle",
  "blocks": [
    {"type": "heading", "level": 1, "text": "Section"},
    {"type": "paragraph", "text": "Body copy."},
    {"type": "bullet_list", "items": ["Point one", {"text": "Point two", "comment": "Verify"}]},
    {"type": "image", "path": "chart.png", "alt": "Revenue trend chart", "caption": "Figure 1. Revenue trend", "width_inches": 5.5, "height_inches": 3.2},
    {"type": "revision", "delete": "Old sentence.", "insert": "Improved sentence."},
    {"type": "table", "headers": ["Metric", "Value"], "rows": [["ARR", "$1.2M"]]}
  ]
}
```

## Quality Bar

- Keep documents editable: use semantic headings, real Word lists, comments,
  tracked changes, images with alt text, and real tables.
- Run `a11y_audit.py` before delivery; fix hard issues such as fake bullets,
  missing image alt text, empty documents, and structural defects.
- Do not overuse tables for normal prose.
- Avoid dense walls of text unless the document type demands it.
- For Google Docs-targeted output, keep the title simple and native-looking and
  run `google_docs_title_sanitize.py --check`.
- When editing existing DOCX packages, preserve package structure: append body
  content before the final `w:sectPr`, keep existing headers/relationships, and
  let watermark helpers allocate a new header part instead of replacing one.
- If preview rendering fails because LibreOffice or a PDF-to-PNG renderer is
  missing, state exactly which verification passed; do not imply visual QA
  passed.
