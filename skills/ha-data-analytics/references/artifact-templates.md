# Analysis Artifact templates

All templates use the same `hope.analysis-artifact.v1` schema. The template
changes narrative order and visual density; it never weakens source, quality,
fallback, or validation requirements.

## report

Use for a durable answer to one decision question.

1. Answer and decision implication
2. Key findings
3. Evidence visuals/tables
4. Recommendations
5. Caveats and methods
6. Data quality, claim validation, and sources

Prefer 1–4 charts and bounded supporting tables.

## dashboard

Use for a repeatable monitoring or driver-exploration view.

1. KPI scorecard with metric definitions and comparison period
2. Trend and segment/drivers
3. Exceptions or guardrails
4. Action callouts
5. Freshness, filters, quality, and sources

Every KPI must expose numerator, denominator, window, and last-refreshed time.
The offline report must preserve values and conclusions without JavaScript.

## data_table

Use when exact lookup, filtering, or row-level auditability is the primary job.

1. Table purpose and grain
2. Column dictionary
3. Bounded rows or summarized partitions
4. Totals/reconciliation
5. Quality flags and source hashes

Do not embed an unbounded extract. Mark sampling and truncation explicitly.

## explainer

Use to explain a model, metric, technical system, or causal hypothesis.

1. Plain-language conclusion
2. Definitions and assumptions
3. Stepwise mechanism or calculation
4. Worked example or evidence table
5. What is observed versus inferred
6. Caveats, validation, and sources

Prefer diagrams represented by semantic text/table fallback. Do not let a
visual carry a claim that disappears when scripts are disabled.
