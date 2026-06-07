---
name: office-pptx
description: "Use when the user asks to create, inspect, verify, polish, or deliver PowerPoint `.pptx` decks, Google Slides-targeted deck artifacts, strategy narratives, operating reviews, pitch decks, teaching decks, section slides, bullet slides, or source-to-PPTX transformations."
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
  - image_generate
  - send_attachment
---

# Office PPTX

Use the bundled scripts in this skill package to produce editable `.pptx`
decks. The builder writes editable text boxes, image placements, metric cards,
tables, timelines, drawn proof charts, and native PowerPoint chart objects. The
skill activation metadata includes `Skill directory`; treat that as `SKILL_DIR`
and run scripts from `SKILL_DIR/scripts/`.

## Story First

Before building slides, identify:

- Audience and decision context.
- Claim spine: what each slide proves.
- Required proof objects: table, metric, comparison, workflow, timeline, or
  narrative slide.
- Visual rhythm: title, section, bullets, two-column, and appendix slides.

Every slide should have one job.

## Workflow

1. Draft the slide list and title-level claims.
2. For explicit control, convert each slide into a structured slide object and
   run:

```bash
python3 "$SKILL_DIR/scripts/check_env.py"
python3 "$SKILL_DIR/scripts/build_pptx.py" --spec spec.json --out output.pptx
python3 "$SKILL_DIR/scripts/inspect_pptx.py" --verify output.pptx
python3 "$SKILL_DIR/scripts/layout_audit.py" output.pptx
```

3. For a Markdown outline, use `# Deck title`, `## Slide title`, `## Section:
   Name`, and bullet lists, then run:

```bash
python3 "$SKILL_DIR/scripts/outline_to_pptx.py" --outline outline.md --out output.pptx
python3 "$SKILL_DIR/scripts/inspect_pptx.py" --verify output.pptx
```

4. If visual QA matters and LibreOffice is available, run:

```bash
python3 "$SKILL_DIR/scripts/render_preview.py" output.pptx
```

5. To append slides to an existing deck without rebuilding it:

```bash
python3 "$SKILL_DIR/scripts/append_pptx.py" --input existing.pptx --spec append.json --out output.pptx
python3 "$SKILL_DIR/scripts/inspect_pptx.py" --verify output.pptx
python3 "$SKILL_DIR/scripts/layout_audit.py" output.pptx --reference existing.pptx
```

6. For targeted edits to an existing deck, patch text in place to preserve the
   source package, theme, layouts, media, and animations:

```bash
python3 "$SKILL_DIR/scripts/patch_pptx.py" --input existing.pptx --patch patch.json --out output.pptx
python3 "$SKILL_DIR/scripts/layout_audit.py" output.pptx --reference existing.pptx
```

Patch shape:

```json
{"replace_text": [{"old": "Old title", "new": "New title"}]}
```

7. For template-following or source-deck workflows, duplicate, drop, or reorder
   source slides without rebuilding their XML:

```bash
python3 "$SKILL_DIR/scripts/duplicate_slide.py" --input template.pptx --slide 2 --out duplicated.pptx
python3 "$SKILL_DIR/scripts/deck_reorder.py" --input duplicated.pptx --order '[2,1,3]' --out reordered.pptx
python3 "$SKILL_DIR/scripts/layout_audit.py" reordered.pptx --reference template.pptx
```

8. After rendering previews, create a contact sheet when reviewing slide rhythm:

```bash
python3 "$SKILL_DIR/scripts/make_contact_sheet.py" --images preview/page-*.png --out contact-sheet.html
```

9. Deliver the `.pptx` path or attach it with `send_attachment`.

## Spec Shape

```json
{
  "title": "Deck title",
  "slides": [
    {"type": "title", "title": "Board Update", "subtitle": "Q2"},
    {"type": "section", "title": "What changed"},
    {"type": "bullets", "title": "Retention improved", "bullets": ["Activation rose", "Churn fell"]},
    {"type": "metrics", "title": "Operating pulse", "metrics": [{"label": "ARR", "value": "$12M", "delta": "+18%"}]},
    {"type": "table", "title": "Options", "headers": ["Option", "Pros", "Risks"], "rows": [["A", "Fast", "Low moat"]]},
    {"type": "timeline", "title": "Launch path", "items": [{"date": "Q1", "label": "Pilot"}, {"date": "Q2", "label": "GA"}]},
    {"type": "chart", "title": "Segment mix", "data": [{"label": "SMB", "value": 42}, {"label": "Enterprise", "value": 58}]},
    {"type": "native_chart", "title": "Native segment mix", "chart_type": "pie", "data": [{"label": "SMB", "value": 42}, {"label": "Enterprise", "value": 58}]},
    {"type": "image", "title": "Product screenshot", "image": "screenshot.png", "caption": "Use verified assets only"},
    {"type": "two_column", "title": "Options", "left": ["Option A"], "right": ["Option B"]}
  ]
}
```

## Quality Bar

- Use concise slide titles with a claim, not just a topic label.
- Keep bullets short; move dense material into an appendix or document.
- Avoid invented logos, marks, screenshots, or metrics. Use verified assets or
  omit them.
- Use `native_chart` when the deck needs a real editable PowerPoint chart
  object; use `chart` only for quick drawn proof-object slides.
- Preserve source-deck style when the user asks for edits to an existing deck;
  use `patch_pptx.py` for text-only edits and minimal local changes rather
  than rebuilding from scratch.
- When duplicating, dropping, or reordering slides, preserve non-slide
  presentation relationships such as slide masters, layouts, themes, and view
  properties; do not rebuild `presentation.xml.rels` from slides only.
- Run the layout audit (`layout_audit.py`) before delivery; fix blank slides,
  missing titles, dense text, and out-of-bounds shapes. Preserve source-deck
  style for template and targeted-edit work.
- If preview rendering fails because LibreOffice or a PDF-to-PNG renderer is
  missing, state exactly which verification passed; do not imply visual QA
  passed.
