# 14 · Capability Evaluation

The Capability Evaluation center runs repeatable synthetic tasks through **real models and the real Hope Agent product path**. It helps answer whether a task was actually completed, how many tools were called, how long it took, how many tokens and dollars it consumed, and whether Goal, Workflow, asynchronous-task, and multi-Agent orchestration remain stable.

This is different from ordinary chat and unit tests: an evaluation actually calls the selected model Provider and executes synthetic tool tasks inside an isolated temporary environment, so it may incur model charges.

**In this chapter**

- [14.1 What it measures—and what it does not prove](#141-what-it-measuresand-what-it-does-not-prove)
- [14.2 Before you run](#142-before-you-run)
- [14.3 Choose a profile](#143-choose-a-profile)
- [14.4 Run a real-model evaluation](#144-run-a-real-model-evaluation)
- [14.5 Progress, cancellation, and retries](#145-progress-cancellation-and-retries)
- [14.6 Read history and per-task results](#146-read-history-and-per-task-results)
- [14.7 Comparisons and trends](#147-comparisons-and-trends)
- [14.8 Evidence, imports, and baselines](#148-evidence-imports-and-baselines)
- [14.9 Cost, credentials, and data boundaries](#149-cost-credentials-and-data-boundaries)
- [14.10 Troubleshooting](#1410-troubleshooting)

---

## 14.1 What it measures—and what it does not prove

**Where to find it**: sidebar → Dashboard → Capability Evaluation.

The built-in Hope Core scenarios currently focus on:

- Goal / Loop creation, progress, stopping, and completion decisions;
- Workflow stages, approvals, recovery, and terminal states;
- concurrent real asynchronous jobs, completion delivery, and attribution;
- Sub-Agent / Agent Team delegation, concurrency, handoff, and compute-matched comparisons;
- mixed end-to-end tasks that combine those capabilities;
- task completion, safety invariants, false completion, budget exhaustion, and infrastructure errors.

The Runner drives tasks through the real Hope Server entry point. Model selection, prompts, tool schemas, permissions, failover, and orchestration state machines all use the product implementation; this is not a text quiz that bypasses Hope Agent and calls a model directly.

It still does not prove that every real-world task is covered. Complete Browser, Office, desktop GUI, external SaaS, and public-benchmark harnesses must be integrated capability by capability. Existing Coding / Domain campaigns appear in unified history, but are never presented as Hope Core scores.

| Track | Calls real models | Starts in the App | Intended use |
| --- | --- | --- | --- |
| Local Hope Core evaluation | Yes | Yes | Local diagnosis and model/version comparison |
| Legacy Coding / Domain campaigns | Depends on the original task | Existing entry points | Read-only display in unified history |
| Deterministic capability evaluation | No | No | Weekly and pre-release code-contract validation |
| Protected Runner evaluation | Yes | No; runs in controlled CI/Runners | Produces signed release evidence |

> A local result never becomes release-gate evidence automatically, no matter how well it scores.

---

## 14.2 Before you run

Confirm the following first:

1. Use a desktop build that includes the evaluation Sidecar. The page should show “Sidecar ready” in the upper-right.
2. In [02 · Models & Providers](02-models-and-providers.md), enable a Provider, register a model, and configure an API key, a valid Auth Profile, or a signed-in Codex account. A configured loopback local model can also be used.
3. Make sure the Provider Base URL is reachable and that the computer has enough disk space and network connectivity.
4. Preferably configure input/output prices for the model so cost budgets and historical cost have meaningful values.

Signed-in Codex OAuth models appear in the selectable list with a permanent “Diagnostic only” label. Before startup, the App verifies that the short-lived token covers the configured maximum duration and refreshes it inside the owner App when necessary. The isolated process receives only the access token, account ID, and expiration time; it never receives OAuth files or a refresh token. If a refreshed token still cannot cover the budget, shorten the maximum duration and generate the plan again. A local Codex result cannot be promoted to release evidence, and protected Runners continue to reject personal Codex OAuth credentials.

If a Provider has multiple Auth Profiles, you can choose a credential profile after selecting the model. Within one experiment, all models from the same Provider must use the same credential profile.

> If a development build reports a missing Sidecar, a developer must first run `pnpm prepare:eval-sidecar`. Release packages include a matching Sidecar with the application.

---

## 14.3 Choose a profile

A profile determines the scenario scope, comparison arms, repetitions, and budget ceilings. Versioned assets can change these ceilings, so the values shown on the App cards are authoritative.

| Profile | Best for | Current focus |
| --- | --- | --- |
| Quick | Checking a newly configured Provider and the main path | Critical smoke/control cases, one repetition, serial by default; three-minute maximum per trial |
| Standard | Routine version or model preflight | A locally compatible control subset of weekly core cases, with case selection |
| Reliability | Recovery, stability, and multi-Agent benefit | Fault/comparison arms and manifest-defined repetitions; currently one model per run |
| Custom | Reproducing one case or narrowing an experiment | Select cases, arms, and 1–5 repetitions from the allowlist; cannot add unregistered tasks |

A practical sequence is:

1. Run **Quick** for a new Provider or model.
2. Run **Standard** after Quick is stable.
3. Use **Reliability** when you suspect a concurrency, recovery, or multi-Agent regression.
4. Use **Custom** to reproduce a known failing case.

Repeated trials expose model randomness. `any-pass@k` means at least one of k attempts succeeded; `all-pass@k` means every attempt succeeded and is the stronger stability signal.

---

## 14.4 Run a real-model evaluation

In Capability Evaluation → Run:

1. **Choose an evaluation profile**.
2. If the profile permits customization, choose scenarios, comparison arms, and repetitions.
3. **Choose real models**. The profile controls the limit, with an App-wide maximum of four. Multiple models become separate Campaigns within one experiment so they can be compared later.
4. **Set hard budgets**:
   - maximum cost in USD;
   - maximum wall time in minutes;
   - concurrency. This controls simultaneous trials; it does not enlarge a scenario's internal Agent or tool budgets.
5. Confirm both “I understand this may incur model charges” and “I understand synthetic tool tasks will execute.”
6. Select **Generate plan**, then check the estimated trial count, model count, cost ceiling, and runtime environment.
7. Select **Start real evaluation**.

Preview and start are bound to one immutable plan. Changing the profile, scenarios, models, credentials, or budgets invalidates the preview, and you must generate it again. This prevents the confirmed plan from differing from what actually runs.

Only one local experiment can be active at a time. Wait for the current run to finish or cancel it before starting another.

---

## 14.5 Progress, cancellation, and retries

After startup succeeds, Run automatically becomes the live workspace for that experiment; you do not need to open History to follow progress. It shows overall progress and duration, pass/failure/infrastructure counts, tokens, model/tool calls, cost, and queued/running/completed state for every Campaign and Trial. Persisted trials can be opened directly for their causal trace. You can switch to History, Compare, or other tabs while the run continues.

The configured duration is a hard ceiling for the whole experiment, not a full allowance for every trial. The immutable plan conservatively shares usable wall time across all trials and reserves time for startup, evidence writes, and process cleanup; Quick applies an additional per-trial cap. A per-trial deadline is reported as Budget exhausted and is not automatically retried as an infrastructure error. After experiment failure, cancellation, or interruption, active rows become Aborted and untouched rows become Not run instead of remaining Running; completed partial results remain available in History.

When the evaluation reaches a terminal state, its result remains in Run. Select “Start new evaluation” to return to configuration. History is primarily for finding prior records, annotations, exports, and retries.

- **Cancel**: select Cancel in the live workspace. Hope first requests a graceful stop and terminates the entire isolated process tree if needed.
- **App closes or the Sidecar exits unexpectedly**: unfinished experiments become `interrupted` and do not resume automatically.
- **Retry**: select “Retry as new experiment” in History details. A retry creates a new experiment linked to its parent; it never overwrites the original calls, cost, or failure evidence.

When a budget is exhausted, the result explicitly records a budget failure/stop. Raising the budget requires a new retry; completed evidence cannot be edited.

---

## 14.6 Read history and per-task results

History includes Hope Core records, local imports, and read-only indexes of existing Coding / Domain campaigns. Open a Hope Core record to inspect every trial.

### Three terminal result classes

| Result | Meaning | What to do |
| --- | --- | --- |
| Passed | The task terminal state and required checks passed | Then inspect efficiency and stability |
| Failed | A valid model/Agent trial ran, but the task or a safety check failed | Inspect the failure class, milestones, and causal events |
| Infra error | The Provider, Runner, environment, or scorer could not form a valid trial | Fix infrastructure first; do not count it as model incapability |

### Per-trial metrics

- **Outcome and checks**: task terminal state, milestones, invariants, judge checks, and blocking status;
- **Time**: wall time, critical path, model-active, tool-active, queue wait, and environment wait;
- **Tokens**: input, output, cache read, and reasoning. Fields absent from the Provider remain empty rather than being fabricated as zero;
- **Tools**: attempted, logical, effective, and retry counts;
- **Orchestration**: model calls/retries, Loop iterations, failovers, Agents/max concurrency, asynchronous jobs, and handoffs;
- **Cost**: calculated from the run's model-price snapshot; unknown prices appear as “—”;
- **Causal trace**: bounded structured events and summaries, without prompts, model response bodies, or full tool arguments.

Interpret results in this order: **task success first, safety and false completion second, then compare time, tools, tokens, and cost only among successful samples**. A fast failure is not an efficiency win.

You can also attach a diagnostic note, pin an important local record for long-term retention, or export an unsigned local diagnostic bundle.

---

## 14.7 Comparisons and trends

### Compare

Open Compare, select a Baseline and Candidate, then calculate. Hope checks whether case/version, arm, model configuration, runtime configuration, assets, and environment are compatible:

- **Exact / Functional**: differences and improvement/regression cues can be shown;
- **Diagnostic only**: values are shown side by side without colors that claim a regression.

Commit SHA is a normal comparison axis, so different commits are not automatically incompatible. Changes to the model, prompt, tool schema, scenario, scorer, or runtime environment may create a baseline break.

### Trends

Open Trends, then choose an anchor experiment and metric. Available metrics include:

- task success and end-to-end yield;
- `any-pass@k` and `all-pass@k`;
- infrastructure error, policy failure, budget exhaustion, and false completion;
- wall time, tool calls, tokens, and cost for successful samples;
- compute-matched multi-Agent uplift.

Task success uses valid trials as its denominator. End-to-end yield uses all scheduled trials, so filtering out infrastructure errors cannot make the result look artificially high.

---

## 14.8 Evidence, imports, and baselines

The integrity label in History is important:

| Label | Meaning | Can create a protected baseline |
| --- | --- | --- |
| Local diagnostic | Local real-model run, unsigned | No |
| Legacy local | Legacy local Coding / Domain record | No |
| Unverified import | Raw JSON or other unsigned import | No |
| Protected · unknown assets | Signature is valid, but this App does not recognize the asset version | No; view only |
| Protected verified | Verified by the built-in trust root, with matching assets and identity | Yes, when tier/status requirements are met |

Baselines provides two import paths:

- **Import signed bundle**: available only when the application includes a trusted public-key registry;
- **Import unsigned JSON**: diagnostic only, without a shield or release eligibility.

Local export is always unsigned. Editing `source` or SHA fields in JSON cannot promote it to protected evidence. Protected baselines come from an organization's dedicated Runner, exact commit SHA, and signature chain—not from a local App run.

---

## 14.9 Cost, credentials, and data boundaries

- **Real charges apply**: requests go directly to the selected Provider. Tokens and estimated cost are bounded by your configured budgets, while the Provider bill remains the cost source of truth.
- **Synthetic tasks only**: built-in scenarios do not construct tasks from real user business data, and evaluation should not use personal production accounts.
- **Isolated execution**: each Campaign receives a temporary HOME, data directory, workspace, port, and Hope Server.
- **Credentials stay out of evidence**: the UI sends only Provider/model/credential references. The local backend resolves the API key or short-lived Codex token, which is not written to plans, databases, command lines, logs, or exports.
- **Local networking is not release-grade isolation proof**: local evidence marks network enforcement as unverified and is diagnostic only.
- **Local storage**: evaluation indexes and artifacts live under `~/.hope-agent/evals/`. Ordinary local artifacts follow retention cleanup; manually pinned records are retained.

---

## 14.10 Troubleshooting

| Symptom | Cause and action |
| --- | --- |
| “Sidecar unavailable” | The Sidecar is missing or its version/digest does not match. Upgrade or reinstall the release desktop app; prepare the evaluation Sidecar in development. |
| No selectable model | The Provider is disabled, the model is not registered, there is no API key/Auth Profile, or Codex is signed out/expired. Fix the configuration or sign in to Codex again under Settings → Providers. |
| Codex token cannot cover the evaluation duration | The isolated Codex process cannot hold a refresh token. Shorten “Maximum duration” and generate the plan again; if it still fails, sign in to Codex again under Settings → Providers. |
| Cost appears as “—” | Model pricing is missing or the Provider did not return usable usage. Diagnosis can continue, but cost budgets and comparisons are incomplete. |
| Start is disabled after a change | The plan is stale. Generate the plan again before starting. |
| An infra error appears | Check Provider reachability, credentials, quota, rate limits, Sidecar health, and local resources. Do not treat it directly as task failure. |
| A run is `interrupted` | The App or Sidecar exited. It cannot resume in place; use “Retry as new experiment” from History. |
| Signed import is unavailable | This build has no usable trust registry, or the signing key/assets are untrusted. You can still import unsigned data for diagnosis. |
| A fully green local run cannot become a protected baseline | This is intentional: local evidence is always local diagnostic; release baselines must come from a protected Runner. |
| Server/Web mode cannot start an evaluation | Real-model run/cancel/retry and local file import/export are desktop-owner operations. HTTP/WS exposes redacted read-only queries only. |

---

## Next steps

- Configure API keys, Auth Profiles, and model pricing → [02 · Models & Providers](02-models-and-providers.md)
- Learn the everyday Goal, Workflow, and Loop controls → [08 · Autonomous Tasks](08-autonomous-tasks.md)
- Learn Sub-Agents, Teams, and concurrent jobs → [09 · Multi-Agent & Scheduled Tasks](09-multi-agent-and-scheduling.md)
- View the ordinary token/cost ledger and system health → [12 · Projects & Insights](12-projects-and-insights.md)
- For the evidence protocol and Runner architecture → [Real-model and complex-task evaluation](../../architecture/live-model-evaluation.md)
