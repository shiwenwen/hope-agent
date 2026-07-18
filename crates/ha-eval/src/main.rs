mod adapters;
mod model;
mod model_adapter;
mod model_app_control;
mod model_fake;
mod model_supervisor;

use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, SecondsFormat, Utc};
use clap::{Parser, Subcommand, ValueEnum};
use ha_eval_spec::{
    case_digest, digest_file, digest_serializable, read_json, sha256_bytes, stable_shard,
    suite_digest, validate_json_schema, validate_policy, validate_suite, write_json,
    ArtifactDigest, CaseResult, EvalEvidence, EvalPlan, EvalPolicy, EvalStatus, EvalTier,
    EvalWaiver, PlannedCase, PlannedSuite, PolicyMode, ShardResult, SuiteManifest,
    EVIDENCE_SCHEMA_VERSION, PLAN_SCHEMA_VERSION, SHARD_SCHEMA_VERSION, WAIVER_SCHEMA_VERSION,
};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug, Parser)]
#[command(name = "hope-agent-eval", version, about)]
struct Cli {
    /// Repository root. Defaults to walking upward from the current directory.
    #[arg(long, global = true)]
    root: Option<PathBuf>,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Validate all committed eval schemas, policies, manifests, and assets.
    Validate,
    /// Build an immutable execution plan for one policy tier and git ref.
    Plan {
        #[arg(long, value_enum)]
        tier: TierArg,
        #[arg(long = "ref")]
        reference: String,
        #[arg(long)]
        output: Option<PathBuf>,
        #[arg(long, value_enum, default_value = "json")]
        format: PlanFormat,
    },
    /// Run one suite shard with per-case subprocess isolation.
    Run {
        #[arg(long)]
        plan: PathBuf,
        #[arg(long)]
        suite: String,
        /// One-based shard in the form i/n, for example 1/4.
        #[arg(long)]
        shard: String,
        #[arg(long)]
        output: PathBuf,
    },
    /// Aggregate shard outputs into release evidence and Markdown summary.
    Aggregate {
        #[arg(long)]
        plan: PathBuf,
        #[arg(long, required = true)]
        inputs: Vec<PathBuf>,
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        summary: PathBuf,
        #[arg(long)]
        waiver: Option<PathBuf>,
    },
    /// Verify evidence against the current policy, suites, commit, and waiver.
    VerifyEvidence {
        #[arg(long)]
        evidence: PathBuf,
        #[arg(long = "ref")]
        reference: String,
        #[arg(long, value_enum)]
        tier: TierArg,
        #[arg(long)]
        tag: Option<String>,
        /// Test-only/local inspection escape hatch; release workflows omit it.
        #[arg(long, hide = true)]
        allow_local: bool,
    },
    /// Run real-model campaigns in a protocol and evidence domain isolated
    /// from deterministic release evals.
    Model {
        #[command(subcommand)]
        command: model::ModelCommands,
    },
    #[command(name = "_run-case", hide = true)]
    RunCase {
        #[arg(long)]
        plan: PathBuf,
        #[arg(long)]
        suite: String,
        #[arg(long)]
        case: String,
        #[arg(long)]
        output: PathBuf,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum TierArg {
    Weekly,
    Release,
}

impl From<TierArg> for EvalTier {
    fn from(value: TierArg) -> Self {
        match value {
            TierArg::Weekly => Self::Weekly,
            TierArg::Release => Self::Release,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum PlanFormat {
    Json,
    Github,
}

#[tokio::main]
async fn main() -> Result<()> {
    model_app_control::install_parent_watchdog_from_env()?;
    let cli = Cli::parse();
    let root = match cli.root {
        Some(root) => root.canonicalize().context("canonicalizing --root")?,
        None => find_repo_root()?,
    };
    match cli.command {
        Commands::Validate => validate_repo(&root),
        Commands::Plan {
            tier,
            reference,
            output,
            format,
        } => command_plan(&root, tier.into(), &reference, output.as_deref(), format),
        Commands::Run {
            plan,
            suite,
            shard,
            output,
        } => command_run(&root, &plan, &suite, &shard, &output),
        Commands::Aggregate {
            plan,
            inputs,
            output,
            summary,
            waiver,
        } => command_aggregate(&root, &plan, &inputs, &output, &summary, waiver.as_deref()),
        Commands::VerifyEvidence {
            evidence,
            reference,
            tier,
            tag,
            allow_local,
        } => command_verify_evidence(
            &root,
            &evidence,
            &reference,
            tier.into(),
            tag.as_deref(),
            allow_local,
        ),
        Commands::Model { command } => model::execute(&root, command).await,
        Commands::RunCase {
            plan,
            suite,
            case,
            output,
        } => command_run_case(&root, &plan, &suite, &case, &output).await,
    }
}

fn find_repo_root() -> Result<PathBuf> {
    let mut current = std::env::current_dir().context("reading current directory")?;
    loop {
        if current.join("Cargo.toml").is_file() && current.join("evals").is_dir() {
            return current
                .canonicalize()
                .context("canonicalizing repository root");
        }
        if !current.pop() {
            bail!("could not find repository root containing Cargo.toml and evals/");
        }
    }
}

fn evals_dir(root: &Path) -> PathBuf {
    root.join("evals")
}

fn policy_path(root: &Path, tier: EvalTier) -> PathBuf {
    evals_dir(root)
        .join("policy")
        .join(format!("{}.json", tier.as_str()))
}

fn suite_dir(root: &Path, id: &str) -> PathBuf {
    evals_dir(root).join("suites").join(id)
}

fn suite_path(root: &Path, id: &str) -> PathBuf {
    suite_dir(root, id).join("suite.json")
}

fn load_policy(root: &Path, tier: EvalTier) -> Result<EvalPolicy> {
    let path = policy_path(root, tier);
    let policy: EvalPolicy = read_json(&path)?;
    validate_policy(&policy)?;
    if policy.tier != tier {
        bail!("policy {} tier does not match file name", policy.id);
    }
    Ok(policy)
}

fn load_suite(root: &Path, id: &str) -> Result<SuiteManifest> {
    let dir = suite_dir(root, id);
    let suite: SuiteManifest = read_json(&dir.join("suite.json"))?;
    validate_suite(&suite, &dir)?;
    if suite.id != id {
        bail!("suite directory {id} contains manifest for {}", suite.id);
    }
    Ok(suite)
}

fn validate_repo(root: &Path) -> Result<()> {
    let evals = evals_dir(root);
    let suite_schema: Value = read_json(&evals.join("schema/suite-v1.schema.json"))?;
    let policy_schema: Value = read_json(&evals.join("schema/policy-v1.schema.json"))?;
    let evidence_schema: Value = read_json(&evals.join("schema/evidence-v1.schema.json"))?;
    for (name, schema) in [
        ("suite", &suite_schema),
        ("policy", &policy_schema),
        ("evidence", &evidence_schema),
    ] {
        if schema.get("$schema").and_then(Value::as_str).is_none() {
            bail!("{name} JSON Schema does not declare $schema");
        }
    }

    let mut policy_suite_ids = BTreeSet::new();
    let mut policy_digests = BTreeMap::new();
    for tier in [EvalTier::Weekly, EvalTier::Release] {
        let path = policy_path(root, tier);
        let raw: Value = read_json(&path)?;
        validate_json_schema(&raw, &policy_schema)
            .with_context(|| format!("validating {} against policy schema", path.display()))?;
        let policy = load_policy(root, tier)?;
        policy_digests.insert(
            format!("{}@{}", policy.id, policy.version),
            digest_serializable(&policy)?,
        );
        for suite in &policy.suites {
            policy_suite_ids.insert(suite.id.clone());
        }
    }

    let mut validated = Vec::new();
    let mut suite_digests = BTreeMap::new();
    for id in policy_suite_ids {
        let path = suite_path(root, &id);
        let raw: Value = read_json(&path)?;
        validate_json_schema(&raw, &suite_schema)
            .with_context(|| format!("validating {} against suite schema", path.display()))?;
        let suite = load_suite(root, &id)?;
        let digest = suite_digest(&suite, &suite_dir(root, &id))?;
        suite_digests.insert(format!("{}@{}", suite.id, suite.version), digest.clone());
        validated.push((id, suite.cases.len(), digest));
    }
    validate_version_lock(
        &evals.join("version-lock.json"),
        &suite_digests,
        &policy_digests,
    )?;
    validated.sort_by(|a, b| a.0.cmp(&b.0));
    for (id, cases, digest) in validated {
        println!("validated {id}: {cases} cases, sha256:{digest}");
    }
    println!("validated deterministic policies and JSON Schemas");
    Ok(())
}

fn validate_version_lock(
    path: &Path,
    suites: &BTreeMap<String, String>,
    policies: &BTreeMap<String, String>,
) -> Result<()> {
    let lock: Value = read_json(path)?;
    if lock.get("schemaVersion").and_then(Value::as_str) != Some("eval-version-lock.v1") {
        bail!("unsupported eval version lock schema");
    }
    for (section, expected) in [("suites", suites), ("policies", policies)] {
        let actual = lock
            .get(section)
            .and_then(Value::as_object)
            .ok_or_else(|| anyhow!("version lock is missing {section}"))?;
        for (versioned_id, digest) in actual {
            let digest = digest
                .as_str()
                .ok_or_else(|| anyhow!("version lock entry {versioned_id} is not a string"))?;
            if digest.len() != 64 || !digest.chars().all(|ch| ch.is_ascii_hexdigit()) {
                bail!("version lock entry {versioned_id} is not a SHA-256 digest");
            }
        }
        for (versioned_id, digest) in expected {
            let locked = actual
                .get(versioned_id)
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("version lock has no entry for {versioned_id}"))?;
            if locked != digest {
                bail!(
                    "{versioned_id} content changed without a version bump; restore the locked content or increment version and append a new lock entry"
                );
            }
        }
    }
    Ok(())
}

fn command_plan(
    root: &Path,
    tier: EvalTier,
    reference: &str,
    output: Option<&Path>,
    format: PlanFormat,
) -> Result<()> {
    validate_git_reference(reference)?;
    let policy = load_policy(root, tier)?;
    let allowed = policy
        .allowed_adapters
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let mut suites = Vec::new();
    for selected in &policy.suites {
        let manifest = load_suite(root, &selected.id)?;
        if !manifest.tiers.contains(&tier) {
            bail!(
                "suite {} does not support {} tier",
                manifest.id,
                tier.as_str()
            );
        }
        if !allowed.contains(&manifest.adapter) {
            bail!("policy {} does not allow suite adapter", policy.id);
        }
        let dir = suite_dir(root, &manifest.id);
        let cases = manifest
            .cases
            .iter()
            .map(|case| {
                Ok(PlannedCase {
                    id: case.id.clone(),
                    path: case.path.clone(),
                    digest: case_digest(case, &dir)?,
                    timeout_seconds: case
                        .timeout_seconds
                        .unwrap_or(manifest.timeout_seconds)
                        .min(900),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        suites.push(PlannedSuite {
            id: manifest.id.clone(),
            version: manifest.version.clone(),
            capability: manifest.capability.clone(),
            adapter: manifest.adapter,
            digest: suite_digest(&manifest, &dir)?,
            shards: manifest.shards,
            cases,
        });
    }
    let policy_digest = digest_serializable(&policy)?;
    let plan = EvalPlan {
        schema_version: PLAN_SCHEMA_VERSION.to_string(),
        reference: reference.to_string(),
        tier,
        policy_id: policy.id,
        policy_version: policy.version,
        policy_digest,
        runner_digest: runner_digest(reference),
        suites,
    };
    if let Some(path) = output {
        write_json(path, &plan)?;
    }
    match format {
        PlanFormat::Json => {
            if output.is_none() {
                println!("{}", serde_json::to_string_pretty(&plan)?);
            }
        }
        PlanFormat::Github => {
            let include = plan
                .suites
                .iter()
                .flat_map(|suite| {
                    (1..=suite.shards).map(move |shard| {
                        serde_json::json!({
                            "suite": suite.id,
                            "shard": format!("{shard}/{}", suite.shards),
                            "shardIndex": shard,
                            "shardTotal": suite.shards,
                            "profile": if suite.adapter == ha_eval_spec::EvalAdapter::MemoryRetrievalScale { "release" } else { "debug" },
                        })
                    })
                })
                .collect::<Vec<_>>();
            println!("{}", serde_json::json!({"include": include}));
        }
    }
    Ok(())
}

fn validate_git_reference(reference: &str) -> Result<()> {
    let reference = reference.trim();
    if !matches!(reference.len(), 40 | 64) || !reference.chars().all(|ch| ch.is_ascii_hexdigit()) {
        bail!("eval ref must be an exact 40- or 64-character commit SHA");
    }
    Ok(())
}

fn runner_digest(reference: &str) -> String {
    sha256_bytes(format!("hope-agent-eval:v1:{reference}").as_bytes())
}

fn parse_shard(value: &str) -> Result<(u16, u16)> {
    let (index, total) = value
        .split_once('/')
        .ok_or_else(|| anyhow!("shard must use i/n form"))?;
    let index = index.parse::<u16>().context("parsing shard index")?;
    let total = total.parse::<u16>().context("parsing shard total")?;
    if total == 0 || index == 0 || index > total {
        bail!("shard must satisfy 1 <= i <= n");
    }
    Ok((index - 1, total))
}

fn command_run(
    root: &Path,
    plan_path: &Path,
    suite_id: &str,
    shard: &str,
    output: &Path,
) -> Result<()> {
    require_network_isolation_when_requested()?;
    let plan: EvalPlan = read_json(plan_path)?;
    validate_plan(root, &plan)?;
    let suite = plan
        .suites
        .iter()
        .find(|suite| suite.id == suite_id)
        .ok_or_else(|| anyhow!("suite {suite_id} is not in plan"))?;
    let (shard_index, shard_total) = parse_shard(shard)?;
    if shard_total != suite.shards {
        bail!(
            "shard total {shard_total} does not match planned {}",
            suite.shards
        );
    }
    let selected = suite
        .cases
        .iter()
        .filter(|case| stable_shard(&case.id, shard_total) == shard_index)
        .collect::<Vec<_>>();
    let started_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let mut results = Vec::new();
    for case in selected {
        let mut result = run_case_subprocess(root, plan_path, suite, case, 1)?;
        if result.status == EvalStatus::InfraError {
            result = run_case_subprocess(root, plan_path, suite, case, 2)?;
        }
        results.push(result);
    }
    let completed_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let duration_ms = wall_duration_ms(
        parse_timestamp(&started_at, "shard startedAt")?,
        parse_timestamp(&completed_at, "shard completedAt")?,
    )?;
    let shard_result = ShardResult {
        schema_version: SHARD_SCHEMA_VERSION.to_string(),
        reference: plan.reference,
        runner_digest: plan.runner_digest,
        suite_id: suite.id.clone(),
        suite_digest: suite.digest.clone(),
        shard_index: shard_index + 1,
        shard_total,
        started_at,
        completed_at,
        duration_ms,
        cases: results,
    };
    write_json(output, &shard_result)
}

fn run_case_subprocess(
    root: &Path,
    plan_path: &Path,
    suite: &PlannedSuite,
    case: &PlannedCase,
    attempt: u8,
) -> Result<CaseResult> {
    let temp = tempfile::tempdir().context("creating case subprocess directory")?;
    let output = temp.path().join("case-result.json");
    let executable = std::env::current_exe().context("resolving eval executable")?;
    let mut command = Command::new(executable);
    command
        .arg("--root")
        .arg(root)
        .arg("_run-case")
        .arg("--plan")
        .arg(plan_path)
        .arg("--suite")
        .arg(&suite.id)
        .arg("--case")
        .arg(&case.id)
        .arg("--output")
        .arg(&output)
        .env("HA_EVAL_NETWORK", "deny")
        .env("HA_EVAL_ATTEMPT", attempt.to_string())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    for (key, _) in std::env::vars() {
        let upper = key.to_ascii_uppercase();
        if upper.ends_with("_API_KEY")
            || upper.ends_with("_TOKEN")
            || upper.contains("OPENAI")
            || upper.contains("ANTHROPIC")
            || upper.contains("PROVIDER_SECRET")
        {
            command.env_remove(key);
        }
    }
    let started = Instant::now();
    let mut child = command.spawn().context("spawning isolated eval case")?;
    let deadline = Duration::from_secs(case.timeout_seconds);
    let process_status = loop {
        if let Some(status) = child.try_wait().context("polling eval case")? {
            break Some(status);
        }
        if started.elapsed() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        thread::sleep(Duration::from_millis(100));
    };
    if process_status.is_some_and(|status| status.success()) && output.is_file() {
        let mut result: CaseResult = read_json(&output)?;
        result.attempt = attempt;
        return Ok(result);
    }
    let error = if process_status.is_none() {
        format!("case timed out after {} seconds", case.timeout_seconds)
    } else {
        format!("case subprocess exited with {process_status:?}")
    };
    Ok(CaseResult {
        suite_id: suite.id.clone(),
        case_id: case.id.clone(),
        case_digest: case.digest.clone(),
        status: EvalStatus::InfraError,
        duration_ms: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
        attempt,
        checks: Vec::new(),
        error: Some(error),
    })
}

async fn command_run_case(
    root: &Path,
    plan_path: &Path,
    suite_id: &str,
    case_id: &str,
    output: &Path,
) -> Result<()> {
    require_network_isolation_when_requested()?;
    if std::env::var("HA_EVAL_NETWORK").as_deref() != Ok("deny") {
        bail!("isolated eval case requires HA_EVAL_NETWORK=deny");
    }
    let plan: EvalPlan = read_json(plan_path)?;
    validate_plan(root, &plan)?;
    let suite = plan
        .suites
        .iter()
        .find(|suite| suite.id == suite_id)
        .ok_or_else(|| anyhow!("suite {suite_id} is not in plan"))?;
    let case = suite
        .cases
        .iter()
        .find(|case| case.id == case_id)
        .ok_or_else(|| anyhow!("case {case_id} is not in suite {suite_id}"))?;
    let attempt = std::env::var("HA_EVAL_ATTEMPT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1);
    let started = Instant::now();
    let result = adapters::run_case(root, suite, case, attempt).await;
    let outcome = match result {
        Ok(mut outcome) => {
            outcome.duration_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
            outcome
        }
        Err(error) => CaseResult {
            suite_id: suite.id.clone(),
            case_id: case.id.clone(),
            case_digest: case.digest.clone(),
            status: EvalStatus::InfraError,
            duration_ms: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
            attempt,
            checks: Vec::new(),
            error: Some(format!("{error:#}")),
        },
    };
    write_json(output, &outcome)
}

fn validate_plan(root: &Path, plan: &EvalPlan) -> Result<()> {
    if plan.schema_version != PLAN_SCHEMA_VERSION {
        bail!("unsupported plan schemaVersion");
    }
    validate_git_reference(&plan.reference)?;
    let policy = load_policy(root, plan.tier)?;
    if plan.policy_id != policy.id
        || plan.policy_version != policy.version
        || plan.policy_digest != digest_serializable(&policy)?
    {
        bail!("plan policy digest/version does not match the repository");
    }
    if plan.runner_digest != runner_digest(&plan.reference) {
        bail!("plan runner digest does not match ref");
    }
    for planned in &plan.suites {
        let suite = load_suite(root, &planned.id)?;
        let dir = suite_dir(root, &planned.id);
        if planned.version != suite.version || planned.digest != suite_digest(&suite, &dir)? {
            bail!("planned suite {} digest/version is stale", planned.id);
        }
        for case in &planned.cases {
            let source = suite
                .cases
                .iter()
                .find(|candidate| candidate.id == case.id)
                .ok_or_else(|| anyhow!("planned case {} no longer exists", case.id))?;
            if case.digest != case_digest(source, &dir)? {
                bail!("planned case {} digest is stale", case.id);
            }
        }
    }
    Ok(())
}

fn command_aggregate(
    root: &Path,
    plan_path: &Path,
    inputs: &[PathBuf],
    output: &Path,
    summary: &Path,
    waiver_path: Option<&Path>,
) -> Result<()> {
    let plan: EvalPlan = read_json(plan_path)?;
    validate_plan(root, &plan)?;
    let policy = load_policy(root, plan.tier)?;
    let shard_files = collect_json_files(inputs)?;
    let mut artifacts = Vec::new();
    let mut found = BTreeMap::<(String, String), CaseResult>::new();
    let mut earliest_started_at = None::<DateTime<Utc>>;
    let mut latest_completed_at = None::<DateTime<Utc>>;
    for path in shard_files {
        let value: Value = match read_json(&path) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if value.get("schemaVersion").and_then(Value::as_str) != Some(SHARD_SCHEMA_VERSION) {
            continue;
        }
        let shard: ShardResult = serde_json::from_value(value)
            .with_context(|| format!("parsing shard result {}", path.display()))?;
        validate_shard(&plan, &shard)?;
        let shard_started_at = parse_timestamp(&shard.started_at, "shard startedAt")?;
        let shard_completed_at = parse_timestamp(&shard.completed_at, "shard completedAt")?;
        earliest_started_at = Some(
            earliest_started_at
                .map(|current| current.min(shard_started_at))
                .unwrap_or(shard_started_at),
        );
        latest_completed_at = Some(
            latest_completed_at
                .map(|current| current.max(shard_completed_at))
                .unwrap_or(shard_completed_at),
        );
        artifacts.push(ArtifactDigest {
            path: artifact_path_label(&path, inputs),
            sha256: digest_file(&path)?,
        });
        for case in shard.cases {
            let key = (case.suite_id.clone(), case.case_id.clone());
            if found.insert(key.clone(), case).is_some() {
                bail!("duplicate shard result for {}/{}", key.0, key.1);
            }
        }
    }
    let mut cases = Vec::new();
    for suite in &plan.suites {
        for case in &suite.cases {
            cases.push(
                found
                    .remove(&(suite.id.clone(), case.id.clone()))
                    .unwrap_or_else(|| CaseResult {
                        suite_id: suite.id.clone(),
                        case_id: case.id.clone(),
                        case_digest: case.digest.clone(),
                        status: EvalStatus::InfraError,
                        duration_ms: 0,
                        attempt: 0,
                        checks: Vec::new(),
                        error: Some("planned case has no shard result".to_string()),
                    }),
            );
        }
    }
    if !found.is_empty() {
        bail!("shard results contain cases that are not in the plan");
    }
    let aggregate_status = aggregate_status(&cases);
    let completed_at = latest_completed_at.unwrap_or_else(Utc::now);
    let started_at = earliest_started_at.unwrap_or(completed_at);
    let duration_ms = wall_duration_ms(started_at, completed_at)?;
    let waiver = waiver_path.map(read_json::<EvalWaiver>).transpose()?;
    if let Some(waiver) = &waiver {
        validate_waiver(waiver, &plan.reference, None)?;
    }
    let evidence = EvalEvidence {
        schema_version: EVIDENCE_SCHEMA_VERSION.to_string(),
        commit_sha: plan.reference.clone(),
        dirty: git_dirty(root),
        source: if std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true") {
            "github_actions".to_string()
        } else {
            "local".to_string()
        },
        app_version: app_version(root)?,
        tier: plan.tier,
        policy_id: plan.policy_id.clone(),
        policy_version: plan.policy_version.clone(),
        policy_mode: policy.mode,
        policy_digest: plan.policy_digest.clone(),
        runner_digest: plan.runner_digest.clone(),
        aggregate_status,
        started_at: started_at.to_rfc3339_opts(SecondsFormat::Millis, true),
        completed_at: completed_at.to_rfc3339_opts(SecondsFormat::Millis, true),
        duration_ms,
        suites: plan.suites.clone(),
        cases,
        artifacts,
        waiver,
    };
    write_json(output, &evidence)?;
    if let Some(parent) = summary.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(summary, evidence_markdown(&evidence))
        .with_context(|| format!("writing summary {}", summary.display()))?;
    println!(
        "aggregated {} cases: {:?} ({:.2}s)",
        evidence.cases.len(),
        evidence.aggregate_status,
        evidence.duration_ms as f64 / 1_000.0
    );
    Ok(())
}

fn collect_json_files(inputs: &[PathBuf]) -> Result<Vec<PathBuf>> {
    fn visit(path: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
        if path.is_dir() {
            for entry in fs::read_dir(path)? {
                visit(&entry?.path(), out)?;
            }
        } else if path.extension().and_then(|value| value.to_str()) == Some("json") {
            out.push(path.to_path_buf());
        }
        Ok(())
    }
    let mut files = Vec::new();
    for input in inputs {
        visit(input, &mut files)?;
    }
    files.sort();
    Ok(files)
}

fn artifact_path_label(path: &Path, inputs: &[PathBuf]) -> String {
    for input in inputs {
        if input.is_dir() {
            if let Ok(relative) = path.strip_prefix(input) {
                return relative.to_string_lossy().replace('\\', "/");
            }
        } else if input == path {
            return input
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("shard.json")
                .to_string();
        }
    }
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("shard.json")
        .to_string()
}

fn validate_shard(plan: &EvalPlan, shard: &ShardResult) -> Result<()> {
    if shard.schema_version != SHARD_SCHEMA_VERSION
        || shard.reference != plan.reference
        || shard.runner_digest != plan.runner_digest
    {
        bail!("shard result ref/runner/schema does not match plan");
    }
    let suite = plan
        .suites
        .iter()
        .find(|suite| suite.id == shard.suite_id)
        .ok_or_else(|| anyhow!("shard suite is not in plan"))?;
    if shard.suite_digest != suite.digest
        || shard.shard_total != suite.shards
        || !(1..=shard.shard_total).contains(&shard.shard_index)
    {
        bail!("shard metadata does not match planned suite {}", suite.id);
    }
    let started_at = parse_timestamp(&shard.started_at, "shard startedAt")?;
    let completed_at = parse_timestamp(&shard.completed_at, "shard completedAt")?;
    if shard.duration_ms != wall_duration_ms(started_at, completed_at)? {
        bail!("shard duration does not match its timestamps");
    }
    for case in &shard.cases {
        let planned = suite
            .cases
            .iter()
            .find(|planned| planned.id == case.case_id)
            .ok_or_else(|| anyhow!("shard case {} is not planned", case.case_id))?;
        if case.suite_id != suite.id
            || case.case_digest != planned.digest
            || stable_shard(&case.case_id, suite.shards) + 1 != shard.shard_index
        {
            bail!("shard case {} metadata/digest is invalid", case.case_id);
        }
    }
    Ok(())
}

fn aggregate_status(cases: &[CaseResult]) -> EvalStatus {
    if cases.iter().any(|case| case.status == EvalStatus::Failed) {
        EvalStatus::Failed
    } else if cases
        .iter()
        .any(|case| case.status == EvalStatus::InfraError)
    {
        EvalStatus::InfraError
    } else {
        EvalStatus::Passed
    }
}

fn git_dirty(root: &Path) -> bool {
    Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(root)
        .output()
        .map(|output| !output.stdout.is_empty())
        .unwrap_or(true)
}

fn parse_timestamp(value: &str, label: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|timestamp| timestamp.with_timezone(&Utc))
        .with_context(|| format!("parsing {label}"))
}

fn wall_duration_ms(started_at: DateTime<Utc>, completed_at: DateTime<Utc>) -> Result<u64> {
    let duration = completed_at.signed_duration_since(started_at);
    if duration < chrono::Duration::zero() {
        bail!("completedAt precedes startedAt");
    }
    u64::try_from(duration.num_milliseconds()).context("wall-clock duration overflow")
}

fn require_network_isolation_when_requested() -> Result<()> {
    if std::env::var("HA_EVAL_REQUIRE_NETWORK_ISOLATION").as_deref() != Ok("1") {
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    {
        let mut interfaces = fs::read_dir("/sys/class/net")?
            .filter_map(|entry| {
                entry
                    .ok()
                    .and_then(|entry| entry.file_name().to_str().map(str::to_string))
            })
            .collect::<Vec<_>>();
        interfaces.sort();
        if interfaces.iter().any(|interface| interface != "lo") {
            bail!(
                "networkPolicy=deny requires a network namespace with only loopback; found {}",
                interfaces.join(", ")
            );
        }
        return Ok(());
    }
    #[cfg(not(target_os = "linux"))]
    bail!("enforced network isolation is only supported by the hosted Linux runner in v1")
}

fn app_version(root: &Path) -> Result<String> {
    let package: Value = read_json(&root.join("package.json"))?;
    package
        .get("version")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("package.json does not contain version"))
}

fn evidence_markdown(evidence: &EvalEvidence) -> String {
    let mut output = format!(
        "# Capability Eval Evidence\n\n- Status: `{:?}`\n- Commit: `{}`\n- Tier: `{:?}`\n- Policy: `{}` `{}` (`{:?}`)\n- Source: `{}`\n- Dirty: `{}`\n- Duration: `{:.2}s`\n\n| Suite | Passed | Failed | Infra |\n|---|---:|---:|---:|\n",
        evidence.aggregate_status,
        evidence.commit_sha,
        evidence.tier,
        evidence.policy_id,
        evidence.policy_version,
        evidence.policy_mode,
        evidence.source,
        evidence.dirty,
        evidence.duration_ms as f64 / 1_000.0,
    );
    for suite in &evidence.suites {
        let suite_cases = evidence
            .cases
            .iter()
            .filter(|case| case.suite_id == suite.id);
        let (mut passed, mut failed, mut infra) = (0, 0, 0);
        for case in suite_cases {
            match case.status {
                EvalStatus::Passed => passed += 1,
                EvalStatus::Failed => failed += 1,
                EvalStatus::InfraError => infra += 1,
            }
        }
        output.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            suite.id, passed, failed, infra
        ));
    }
    if let Some(waiver) = &evidence.waiver {
        output.push_str(&format!(
            "\n## Audited waiver\n\nApproved by `{}` for `{}`: {}\n",
            waiver.approved_by, waiver.tag, waiver.reason
        ));
    }
    output
}

fn command_verify_evidence(
    root: &Path,
    evidence_path: &Path,
    reference: &str,
    tier: EvalTier,
    tag: Option<&str>,
    allow_local: bool,
) -> Result<()> {
    let raw: Value = read_json(evidence_path)?;
    let schema: Value = read_json(&evals_dir(root).join("schema/evidence-v1.schema.json"))?;
    validate_json_schema(&raw, &schema).context("validating evidence JSON Schema")?;
    let evidence: EvalEvidence = serde_json::from_value(raw)?;
    if evidence.schema_version != EVIDENCE_SCHEMA_VERSION
        || evidence.commit_sha != reference
        || evidence.tier != tier
        || evidence.runner_digest != runner_digest(reference)
    {
        bail!("evidence schema/ref/tier/runner does not match release request");
    }
    if evidence.dirty {
        bail!("release evidence was produced from a dirty worktree");
    }
    if !allow_local && evidence.source != "github_actions" {
        bail!("release evidence must come from GitHub Actions");
    }
    let expected_app_version = app_version(root)?;
    if evidence.app_version != expected_app_version {
        bail!(
            "evidence app version {} does not match repository version {}",
            evidence.app_version,
            expected_app_version
        );
    }
    if let Some(tag) = tag {
        let tag_version = tag.strip_prefix('v').unwrap_or(tag);
        if tag_version != evidence.app_version {
            bail!(
                "evidence app version {} does not match release tag {}",
                evidence.app_version,
                tag
            );
        }
    }
    let policy = load_policy(root, tier)?;
    if evidence.policy_id != policy.id
        || evidence.policy_version != policy.version
        || evidence.policy_mode != policy.mode
        || evidence.policy_digest != digest_serializable(&policy)?
    {
        bail!("evidence policy does not match repository policy");
    }
    let expected_plan = rebuild_expected_plan(root, reference, &policy)?;
    if evidence.suites != expected_plan.suites {
        bail!("evidence suite/case digests do not match repository assets");
    }
    let expected_case_count = evidence
        .suites
        .iter()
        .map(|suite| suite.cases.len())
        .sum::<usize>();
    if evidence.cases.len() != expected_case_count {
        bail!("evidence does not contain every planned case exactly once");
    }
    let mut seen = BTreeSet::new();
    for case in &evidence.cases {
        let key = (case.suite_id.clone(), case.case_id.clone());
        if !seen.insert(key.clone()) {
            bail!("duplicate evidence case {}/{}", key.0, key.1);
        }
        let planned = evidence
            .suites
            .iter()
            .find(|suite| suite.id == case.suite_id)
            .and_then(|suite| {
                suite
                    .cases
                    .iter()
                    .find(|planned| planned.id == case.case_id)
            })
            .ok_or_else(|| anyhow!("evidence contains unplanned case"))?;
        if case.case_digest != planned.digest {
            bail!("evidence case digest mismatch for {}", case.case_id);
        }
    }
    if evidence.aggregate_status != aggregate_status(&evidence.cases) {
        bail!("evidence aggregate status does not match its case results");
    }
    let started_at = parse_timestamp(&evidence.started_at, "evidence startedAt")?;
    let completed_at = parse_timestamp(&evidence.completed_at, "evidence completedAt")?;
    let expected_duration = wall_duration_ms(started_at, completed_at)?;
    if evidence.duration_ms != expected_duration {
        bail!("evidence duration does not match its timestamps");
    }
    let expected_artifacts = evidence
        .suites
        .iter()
        .map(|suite| usize::from(suite.shards))
        .sum::<usize>();
    if evidence.artifacts.len() != expected_artifacts {
        bail!("evidence does not contain one artifact digest per planned shard");
    }
    let mut artifact_paths = BTreeSet::new();
    for artifact in &evidence.artifacts {
        if !artifact_paths.insert(artifact.path.as_str()) {
            bail!(
                "evidence contains duplicate artifact path {}",
                artifact.path
            );
        }
        if artifact.sha256.len() != 64 || !artifact.sha256.chars().all(|ch| ch.is_ascii_hexdigit())
        {
            bail!("evidence artifact {} has an invalid SHA-256", artifact.path);
        }
    }
    let failed_suites = policy_failures(&policy, &evidence);
    if let Some(waiver) = &evidence.waiver {
        if tier != EvalTier::Release {
            bail!("waivers are only valid for release-tier evidence");
        }
        let release_tag = tag.ok_or_else(|| anyhow!("waived evidence requires a release tag"))?;
        validate_waiver(waiver, reference, Some(release_tag))?;
        let policy_suites = policy
            .suites
            .iter()
            .map(|suite| suite.id.as_str())
            .collect::<BTreeSet<_>>();
        let mut declared = BTreeSet::new();
        for suite in &waiver.suites {
            if !policy_suites.contains(suite.as_str()) {
                bail!("waiver contains unknown suite {suite}");
            }
            if !declared.insert(suite.as_str()) {
                bail!("waiver contains duplicate suite {suite}");
            }
        }
        let waived = waiver.suites.iter().collect::<BTreeSet<_>>();
        if failed_suites.iter().any(|suite| !waived.contains(suite)) {
            bail!("waiver does not cover every failed suite");
        }
    }
    if policy.mode == PolicyMode::Enforce && !failed_suites.is_empty() && evidence.waiver.is_none()
    {
        bail!(
            "enforced eval policy failed suites: {}",
            failed_suites.join(", ")
        );
    }
    println!(
        "verified {:?} evidence for {} (policy {:?}, failed suites: {})",
        evidence.aggregate_status,
        reference,
        policy.mode,
        if failed_suites.is_empty() {
            "none".to_string()
        } else {
            failed_suites.join(", ")
        }
    );
    Ok(())
}

fn rebuild_expected_plan(root: &Path, reference: &str, policy: &EvalPolicy) -> Result<EvalPlan> {
    let mut suites = Vec::new();
    for selected in &policy.suites {
        let manifest = load_suite(root, &selected.id)?;
        let dir = suite_dir(root, &selected.id);
        suites.push(PlannedSuite {
            id: manifest.id.clone(),
            version: manifest.version.clone(),
            capability: manifest.capability.clone(),
            adapter: manifest.adapter,
            digest: suite_digest(&manifest, &dir)?,
            shards: manifest.shards,
            cases: manifest
                .cases
                .iter()
                .map(|case| {
                    Ok(PlannedCase {
                        id: case.id.clone(),
                        path: case.path.clone(),
                        digest: case_digest(case, &dir)?,
                        timeout_seconds: case
                            .timeout_seconds
                            .unwrap_or(manifest.timeout_seconds)
                            .min(900),
                    })
                })
                .collect::<Result<_>>()?,
        });
    }
    Ok(EvalPlan {
        schema_version: PLAN_SCHEMA_VERSION.to_string(),
        reference: reference.to_string(),
        tier: policy.tier,
        policy_id: policy.id.clone(),
        policy_version: policy.version.clone(),
        policy_digest: digest_serializable(policy)?,
        runner_digest: runner_digest(reference),
        suites,
    })
}

fn policy_failures(policy: &EvalPolicy, evidence: &EvalEvidence) -> Vec<String> {
    let mut failures = Vec::new();
    for selected in &policy.suites {
        let cases = evidence
            .cases
            .iter()
            .filter(|case| case.suite_id == selected.id)
            .collect::<Vec<_>>();
        let pass_rate = cases
            .iter()
            .filter(|case| case.status == EvalStatus::Passed)
            .count() as f64
            / cases.len().max(1) as f64;
        if pass_rate < selected.min_pass_rate
            || cases
                .iter()
                .any(|case| case.status == EvalStatus::InfraError)
        {
            failures.push(selected.id.clone());
        }
    }
    failures
}

fn validate_waiver(waiver: &EvalWaiver, reference: &str, tag: Option<&str>) -> Result<()> {
    if waiver.schema_version != WAIVER_SCHEMA_VERSION
        || waiver.commit_sha != reference
        || waiver.reason.trim().is_empty()
        || waiver.suites.is_empty()
        || waiver.approved_by.trim().is_empty()
        || waiver.workflow_run_id.trim().is_empty()
    {
        bail!("waiver is incomplete or targets a different commit");
    }
    if tag.is_some_and(|tag| waiver.tag != tag) {
        bail!("waiver tag does not match the release tag");
    }
    Ok(())
}
