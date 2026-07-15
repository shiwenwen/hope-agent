# Data quality checks

Use evidence-appropriate checks. Mark absent or irrelevant checks
`not_applicable`; never silently omit a likely failure mode.

| Area | Minimum question | Blocking examples |
|---|---|---|
| Freshness | Does the source cover the requested period and update cadence? | Missing latest period; partial current period compared with complete prior period |
| Schema | Are types, units, enums, and column meanings stable? | Numeric parsed as text; currency/unit changes |
| Missingness | Which key fields are null and is null meaningful? | Missing numerator/denominator, segment, timestamp, or join key |
| Duplicates | Is the expected key unique at the intended grain? | Duplicate events/entities inflate a metric |
| Grain | Does each row represent the same unit used in formulas? | Mixing event, user, account, and daily aggregates |
| Denominator | Is eligibility explicit and stable across periods/segments? | Rate compared with a different eligible population |
| Joins | What are match rates, fan-out, and unmatched populations? | Many-to-many fan-out; material unmatched rows |
| Coverage | Are dates, geographies, platforms, and cohorts represented? | Segment omitted or instrumentation unavailable |
| Sample | Is sample size adequate and selection biased? | Tiny cohorts presented as stable conclusions |
| Outliers | Are extreme values real, capped, winsorized, or erroneous? | One bad row drives the conclusion |
| Time | Are timezone, week boundary, late arrivals, and partial periods aligned? | UTC/local mismatch; incomplete trailing day/week |
| Conflicts | Do independent sources or definitions disagree? | Dashboard and extract use different metric definitions |

## Status rules

- `passed`: deterministic check ran and met the stated criterion.
- `warning`: useful analysis remains possible, but interpretation is limited.
- `failed`: the data cannot support the affected claim safely.
- `not_applicable`: the check does not apply, with a reason.

Set `blocking: true` when failure invalidates a key metric, comparison, join, or
decision. Any unresolved blocking failure requires Artifact status `partial` or
`blocked`.

## Minimum audit record

Each check records:

- dataset/source ID;
- check name and status;
- deterministic method or query;
- observed value and expected criterion;
- affected metrics/findings;
- whether it blocks `ready`;
- remediation or caveat.

Do not use the model's prose assessment alone as a deterministic data-quality
check. The prose may explain a computed result, not replace it.
