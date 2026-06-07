---
name: office-xlsx
description: "Use when the user asks to create, inspect, verify, analyze, format, or deliver Excel `.xlsx` workbooks, Google Sheets-targeted spreadsheet artifacts, trackers, budgets, models, tables, dashboards, formulas, CSV/TSV-to-XLSX conversions, or spreadsheet-ready data packs."
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

# Office XLSX

Use the bundled scripts in this skill package to produce editable `.xlsx`
workbooks. The builder writes formulas, styles, bar/column/line/pie charts,
real Excel tables, data validation, conditional formatting, and workbook
recalculation hints. The skill activation metadata includes `Skill directory`;
treat that as `SKILL_DIR` and run scripts from `SKILL_DIR/scripts/`.

## Workbook Shape

For nontrivial workbooks, prefer:

1. Summary or dashboard sheet first.
2. Inputs / assumptions sheet next.
3. Detail or source data sheets after that.
4. Checks sheet only when formulas, reconciliations, or model integrity matter.

## Workflow

1. Normalize source data before writing the workbook.
2. Use formulas for derived values instead of hardcoded calculated outputs.
   Strings beginning with `=` are written as Excel formulas.
3. For structured workbook creation, create a JSON spec in the working
   directory and run:

```bash
python3 "$SKILL_DIR/scripts/check_env.py"
python3 "$SKILL_DIR/scripts/build_xlsx.py" --spec spec.json --out output.xlsx
python3 "$SKILL_DIR/scripts/inspect_xlsx.py" --verify output.xlsx
python3 "$SKILL_DIR/scripts/formula_audit.py" output.xlsx
```

4. For CSV/TSV conversion, run one or more inputs into one workbook:

```bash
python3 "$SKILL_DIR/scripts/csv_to_xlsx.py" --input data.csv --input lookup.tsv --sheet Data --sheet Lookup --out output.xlsx
python3 "$SKILL_DIR/scripts/inspect_xlsx.py" --verify output.xlsx
```

5. If visual QA matters and LibreOffice is available, run:

```bash
python3 "$SKILL_DIR/scripts/render_preview.py" output.xlsx
```

6. To patch an existing workbook without rebuilding it, create a patch JSON and
   run:

```bash
python3 "$SKILL_DIR/scripts/patch_xlsx.py" --input existing.xlsx --patch patch.json --out output.xlsx
python3 "$SKILL_DIR/scripts/inspect_xlsx.py" --verify output.xlsx
python3 "$SKILL_DIR/scripts/formula_audit.py" output.xlsx --write-cache cached-output.xlsx
```

Patch actions:

```json
{
  "actions": [
    {"action": "append_rows", "sheet": "Data", "rows": [["New", 123, "=B2*2"]]},
    {"action": "set_cell", "sheet": "Summary", "cell": "B2", "value": "=SUM(Data!B:B)"}
  ]
}
```

7. When formulas matter, use `formula_audit.py --write-cache` and deliver the
   cached workbook unless the audit reports unsupported formulas that require
   Excel/LibreOffice recalculation. For broad formula coverage, run:

```bash
python3 "$SKILL_DIR/scripts/recalculate_xlsx.py" output.xlsx --out recalculated.xlsx
python3 "$SKILL_DIR/scripts/inspect_xlsx.py" --verify recalculated.xlsx
```

8. Deliver the `.xlsx` path or attach it with `send_attachment`.

## Spec Shape

```json
{
  "title": "Workbook title",
  "sheets": [
    {
      "name": "Summary",
      "rows": [["Metric", "Value"], ["Revenue", 1200000], ["Margin", "=B2*0.42"]],
      "tables": [{"name": "SummaryTable", "ref": "A1:B3"}],
      "data_validations": [{"range": "A2:A10", "type": "list", "formula1": ["Revenue", "Margin"]}],
      "conditional_formats": [{"range": "B2:B10", "type": "colorScale"}],
      "charts": [
        {"type": "column", "title": "Summary", "categories": "$A$2:$A$3", "values": "$B$2:$B$3", "anchor": "D2"},
        {"type": "line", "title": "Trend", "categories": "$A$2:$A$3", "values": "$B$2:$B$3", "anchor": "D18"}
      ],
      "column_formats": ["text", "currency"],
      "column_widths": [24, 16],
      "freeze_top_row": true,
      "autofilter": true
    }
  ]
}
```

## Quality Bar

- Keep important values visible; avoid tiny columns and clipped headers.
- Use one workbook, not many disconnected CSV-like sheets, when relationships
  between tabs matter.
- Prefer real Excel tables, filters, validations, conditional formats, and
  charts when the workbook is meant to be used repeatedly.
- Keep formulas editable, audit formulas before delivery, and write cached
  values for supported formulas when possible. Supported audit functions include
  common arithmetic, comparisons, `SUM`, `AVERAGE`, `MIN`, `MAX`, `COUNT`,
  `MEDIAN`, `ROUND`, `ABS`, and simple `IF`.
- When patching an existing workbook, preserve unrelated package parts and rerun
  `inspect_xlsx.py` plus `formula_audit.py`; use `recalculate_xlsx.py` when the
  formula surface exceeds the bundled evaluator.
- If preview rendering fails because LibreOffice or a PDF-to-PNG renderer is
  missing, state exactly which verification passed; do not imply visual QA
  passed.
