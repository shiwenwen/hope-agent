# AnalysisArtifactV1

The canonical JSON schema identifier is `hope.analysis-artifact.v1`. The Core
importer rejects another identifier, an empty `question`, an invalid status, or
charts without dataset/source bindings.

## Required top-level fields

```json
{
  "schemaVersion": "hope.analysis-artifact.v1",
  "question": "Why did activation fall in Q2?",
  "audience": "Product leadership",
  "decision": "Choose the first remediation experiment",
  "status": "ready",
  "metricDefinitions": [],
  "timeRange": {"start": "2026-04-01", "end": "2026-06-30", "timezone": "UTC"},
  "filters": [],
  "grain": "week",
  "datasets": [],
  "findings": [],
  "recommendations": [],
  "caveats": [],
  "blocks": [],
  "charts": [],
  "tables": [],
  "staticFallbacks": [],
  "sources": [],
  "dataQuality": [],
  "claimValidation": []
}
```

`status` is exactly `ready`, `partial`, or `blocked`.

## Conventions

Use stable IDs within one payload. References use those IDs rather than array
positions.

Metric definition:

```json
{
  "id": "activation_rate",
  "label": "Activation rate",
  "formula": "activated_new_users / eligible_new_users",
  "numerator": "activated_new_users",
  "denominator": "eligible_new_users",
  "unit": "percent",
  "window": "within 7 days of signup"
}
```

Bounded dataset:

```json
{
  "id": "activation_weekly",
  "sourceIds": ["warehouse_extract_q2"],
  "columns": ["week", "eligible", "activated", "activation_rate"],
  "rowCount": 13,
  "rows": [],
  "truncated": false,
  "calculationRef": "analysis.py#activation_weekly"
}
```

Source:

```json
{
  "id": "warehouse_extract_q2",
  "label": "Q2 activation extract",
  "type": "csv",
  "path": "data/activation_q2.csv",
  "sha256": "...",
  "retrievedAt": "2026-07-14T08:00:00Z",
  "accessScope": "private",
  "redistributable": false
}
```

Quality result:

```json
{
  "id": "dq_unique_signup",
  "datasetId": "activation_weekly",
  "check": "duplicate_key",
  "status": "passed",
  "method": "count(*) - count(distinct signup_id)",
  "observed": 0,
  "blocking": true
}
```

Finding and claim validation:

```json
{
  "id": "finding_mobile_mix",
  "summary": "A mobile traffic mix shift explains 62% of the decline.",
  "datasetIds": ["activation_by_platform"],
  "sourceIds": ["warehouse_extract_q2"],
  "confidence": 0.86
}
```

```json
{
  "claim": "Mobile mix explains 62% of the decline",
  "metric": "activation_rate",
  "denominator": "eligible_new_users",
  "verdict": "supported",
  "method": "Oaxaca-style mix decomposition",
  "sourceIds": ["warehouse_extract_q2"],
  "confidence": 0.86
}
```

Chart:

```json
{
  "id": "activation_trend",
  "type": "line",
  "title": "Weekly activation rate",
  "dataset": "activation_weekly",
  "sourceId": "warehouse_extract_q2",
  "x": "week",
  "y": "activation_rate",
  "unit": "percent",
  "fallbackId": "activation_trend_table"
}
```

Presentation table (preferred for reports):

```json
{
  "id": "activation_platform_table",
  "title": "Activation by platform",
  "datasetId": "activation_weekly",
  "columns": ["platform", "eligible_users", "activated_users", "activation_rate_percent"],
  "columnFormats": {
    "activation_rate_percent": {"unit": "percent", "scale": "points"}
  },
  "rows": [
    {
      "platform": "web",
      "eligible_users": 50,
      "activated_users": 32,
      "activation_rate_percent": 64.0
    }
  ]
}
```

Use table-level `columns` and bounded `rows` when the reader should see a
curated view rather than every calculation column in the bound dataset. Values
must remain reconcilable to the dataset; this is a presentation projection,
not a second calculation path. Use `columnFormats` for semantic formatting;
`unit: percent` with `scale: fraction` converts `0.64` to `64%`, while
`scale: points` renders `64` as `64%`. Without explicit semantic metadata the
renderer preserves numeric values and does not infer units from column names.

Fallbacks may be a table ID, an accessible textual description, or a local
image/SVG asset. They must preserve the conclusion when scripts are disabled.

## Report blocks

Blocks are renderer-owned narrative inputs, not arbitrary executable HTML.

```json
{
  "id": "answer",
  "type": "narrative",
  "title": "Answer",
  "body": "Activation fell primarily because ...",
  "findingIds": ["finding_mobile_mix"]
}
```

Prefer Markdown in `body`. Never embed remote scripts, iframes, forms, or
network fetches.
