use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use chrono::{DateTime, SecondsFormat, Utc};
use clap::{Subcommand, ValueEnum};
use ha_eval_spec::app::{
    app_profile_digest, validate_app_control_envelope, validate_app_plan, validate_app_profile,
    validate_app_request, validate_trust_registry, AppArmMode, AppControlCommand,
    AppControlEnvelope, AppControlEvent, AppDebugRetention, AppEvalCaseCatalog, AppEvalConsent,
    AppEvalSuiteCatalog, AppExecutionProfile, AppModelSelection, AppResolvedCampaign,
    AppResolvedModelBinding, AppSuiteRequest, EvalAppPlan, EvalAppProfile, EvalAppRunRequest,
    EvidenceTrustRegistry, NetworkEnforcement, RuntimeEnvironmentSnapshot,
    APP_CONTROL_PROTOCOL_VERSION, APP_MAX_TRIALS, APP_PLAN_SCHEMA_VERSION,
    APP_REQUEST_SCHEMA_VERSION,
};
use ha_eval_spec::model::{
    aggregate_counts, aggregate_metrics, aggregate_model_status, digest_model_profile,
    model_case_digest, model_runner_digest, model_suite_digest, read_json_or_yaml,
    reject_embedded_secrets, scenario_component_digests, scenario_digest, stable_trial_id,
    stable_trial_seed, strictest_budget, validate_evidence_shape, validate_model_policy,
    validate_model_suite, validate_model_trace, validate_scenario, AttributionCompleteness,
    CampaignBudget, CostMetrics, ExecutionMode, FaultProfile, LiveAgentScenario,
    ModelCampaignAdapter, ModelCampaignCaseSpec, ModelCampaignEvidence, ModelCampaignOutcome,
    ModelCampaignPlan, ModelCampaignPolicy, ModelCampaignSource, ModelCampaignSuite,
    ModelCampaignTier, ModelCampaignWaiver, ModelPolicySuite, ModelProfile, ModelShardResult,
    ModelTrialResult, NetworkPolicy, OrchestrationMetrics, PlannedModelCase, PlannedModelSuite,
    PlannedModelTrial, RunnerClass, TimingMetrics, TokenMetrics, ToolMetrics, TraceSummary,
    UserSimulatorKind, EVIDENCE_SCHEMA_VERSION, PLAN_SCHEMA_VERSION, POLICY_SCHEMA_VERSION,
    SCENARIO_SCHEMA_VERSION, SHARD_SCHEMA_VERSION, SUITE_SCHEMA_VERSION, TRIAL_SCHEMA_VERSION,
    WAIVER_SCHEMA_VERSION,
};
use ha_eval_spec::{
    digest_file, digest_serializable, read_json, resolve_contained, sha256_bytes, stable_shard,
    validate_json_schema, write_json, ArtifactDigest, EvalStatus, PolicyMode,
};
use ring::signature::Ed25519KeyPair;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const RUNTIME_CONFIG_DIGEST_ENV: &str = "HA_MODEL_EVAL_RUNTIME_CONFIG_DIGEST";
const APP_CONTROL_ENV: &str = "HA_MODEL_EVAL_APP_CONTROL";
const APP_PLAN_ENV: &str = "HA_MODEL_EVAL_APP_PLAN";

#[derive(Debug, Subcommand)]
pub enum ModelCommands {
    /// Validate real-model schemas, policies, suites, scenarios, and locks.
    Validate,
    /// Build an immutable real-model campaign plan for an exact commit SHA.
    Plan {
        #[arg(long, value_enum)]
        tier: ModelTierArg,
        #[arg(long = "ref")]
        reference: String,
        #[arg(long)]
        output: Option<PathBuf>,
        #[arg(long, value_enum, default_value = "json")]
        format: ModelPlanFormat,
    },
    /// Execute one real-model suite shard. This may incur Provider charges.
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
        /// Explicit acknowledgement that cloud models may incur charges.
        #[arg(long)]
        confirm_model_costs: bool,
    },
    /// Run a zero-cost fake-Provider smoke through a real Hope server.
    Smoke {
        /// Built headless Hope server. Defaults to target/debug/hope-agent-server.
        #[arg(long)]
        server_bin: Option<PathBuf>,
        /// Schema-valid trial evidence written even when a hard smoke check fails.
        #[arg(long)]
        output: PathBuf,
    },
    /// Zero-cost full App-control smoke through Sidecar and real Hope Server.
    AppSmoke {
        /// Sidecar executable. Defaults to the currently running binary.
        #[arg(long)]
        sidecar: Option<PathBuf>,
        /// Built Hope server. Defaults to target/debug/hope-agent-server.
        #[arg(long)]
        server_bin: Option<PathBuf>,
    },
    /// Aggregate model shard outputs into isolated campaign evidence.
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
    /// Verify model evidence against current assets and release policy.
    VerifyEvidence {
        #[arg(long)]
        evidence: PathBuf,
        #[arg(long = "ref")]
        reference: String,
        #[arg(long, value_enum)]
        tier: ModelTierArg,
        #[arg(long)]
        tag: Option<String>,
        /// Local inspection escape hatch. Release workflows must omit it.
        #[arg(long, hide = true)]
        allow_local: bool,
    },
    /// Create a detached Ed25519-signed archive for offline App import.
    BundleExport {
        #[arg(long)]
        evidence: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        repository: String,
        #[arg(long)]
        workflow: String,
        #[arg(long)]
        workflow_run_id: String,
        #[arg(long, default_value = "model-eval")]
        environment: String,
        #[arg(long)]
        key_id: String,
        #[arg(long, default_value = "MODEL_EVAL_EVIDENCE_SIGNING_KEY_PKCS8_B64")]
        signing_key_env: String,
    },
    /// Verify an offline signed bundle against the repository trust registry.
    BundleVerify {
        #[arg(long)]
        bundle: PathBuf,
        #[arg(long)]
        trust_registry: Option<PathBuf>,
    },
    /// List the versioned profiles exposed by the desktop evaluation center.
    #[command(name = "_app-profiles", hide = true)]
    AppProfiles,
    /// Resolve a credential-free desktop request into an immutable local plan.
    #[command(name = "_app-plan", hide = true)]
    AppPlan {
        #[arg(long)]
        request: PathBuf,
        #[arg(long)]
        models: PathBuf,
        #[arg(long)]
        runtime: PathBuf,
        #[arg(long = "ref")]
        reference: String,
        #[arg(long)]
        app_version: String,
        #[arg(long)]
        dirty: bool,
        #[arg(long)]
        output: PathBuf,
    },
    /// Desktop evaluation-center JSONL control process.
    #[command(name = "app-control", hide = true)]
    AppControl,
    /// Registered loopback supervisor for durable process-restart scenarios.
    #[command(name = "_supervise-server", hide = true)]
    SuperviseServer {
        #[arg(long)]
        server_bin: PathBuf,
        #[arg(long)]
        bind: String,
        #[arg(long)]
        control_bind: String,
        /// Read the Provider/server/supervisor credential envelope from the
        /// anonymous stdin stream instead of inheriting secrets in env.
        #[arg(long)]
        credentials_stdin: bool,
    },
    #[command(name = "_run-trial", hide = true)]
    RunTrial {
        #[arg(long)]
        plan: PathBuf,
        #[arg(long)]
        trial: String,
        #[arg(long)]
        output: PathBuf,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ModelTierArg {
    Nightly,
    Weekly,
    Release,
    Monthly,
}

impl From<ModelTierArg> for ModelCampaignTier {
    fn from(value: ModelTierArg) -> Self {
        match value {
            ModelTierArg::Nightly => Self::Nightly,
            ModelTierArg::Weekly => Self::Weekly,
            ModelTierArg::Release => Self::Release,
            ModelTierArg::Monthly => Self::Monthly,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ModelPlanFormat {
    Json,
    Github,
}

pub async fn execute(root: &Path, command: ModelCommands) -> Result<()> {
    match command {
        ModelCommands::Validate => validate_repo(root),
        ModelCommands::Plan {
            tier,
            reference,
            output,
            format,
        } => command_plan(root, tier.into(), &reference, output.as_deref(), format),
        ModelCommands::Run {
            plan,
            suite,
            shard,
            output,
            confirm_model_costs,
        } => command_run(root, &plan, &suite, &shard, &output, confirm_model_costs),
        ModelCommands::Smoke { server_bin, output } => {
            command_smoke(root, server_bin.as_deref(), &output).await
        }
        ModelCommands::AppSmoke {
            sidecar,
            server_bin,
        } => command_app_control_smoke(root, sidecar.as_deref(), server_bin.as_deref()).await,
        ModelCommands::Aggregate {
            plan,
            inputs,
            output,
            summary,
            waiver,
        } => command_aggregate(root, &plan, &inputs, &output, &summary, waiver.as_deref()),
        ModelCommands::VerifyEvidence {
            evidence,
            reference,
            tier,
            tag,
            allow_local,
        } => command_verify_evidence(
            root,
            &evidence,
            &reference,
            tier.into(),
            tag.as_deref(),
            allow_local,
        ),
        ModelCommands::BundleExport {
            evidence,
            output,
            repository,
            workflow,
            workflow_run_id,
            environment,
            key_id,
            signing_key_env,
        } => command_bundle_export(
            &evidence,
            &output,
            &repository,
            &workflow,
            &workflow_run_id,
            &environment,
            &key_id,
            &signing_key_env,
        ),
        ModelCommands::BundleVerify {
            bundle,
            trust_registry,
        } => {
            let trust =
                trust_registry.unwrap_or_else(|| live_root(root).join("trust/evidence-keys.json"));
            let verified = ha_core::evaluation::verify_evidence_bundle(&bundle, &trust)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "status": "verified",
                    "keyId": verified.key_id,
                    "campaignId": verified.evidence.campaign_id,
                    "commitSha": verified.evidence.commit_sha,
                    "tier": verified.evidence.tier,
                }))?
            );
            Ok(())
        }
        ModelCommands::AppProfiles => {
            println!(
                "{}",
                serde_json::to_string_pretty(&list_app_profiles(root)?)?
            );
            Ok(())
        }
        ModelCommands::AppPlan {
            request,
            models,
            runtime,
            reference,
            app_version,
            dirty,
            output,
        } => {
            let request = read_json(&request)?;
            let models = read_json::<Vec<AppResolvedModelBinding>>(&models)?;
            let runtime = read_json(&runtime)?;
            let plan = build_app_plan(
                root,
                &request,
                &models,
                &reference,
                dirty,
                &app_version,
                runtime,
            )?;
            write_json(&output, &plan)
        }
        ModelCommands::AppControl => crate::model_app_control::run(root).await,
        ModelCommands::RunTrial {
            plan,
            trial,
            output,
        } => command_run_trial(root, &plan, &trial, &output).await,
        ModelCommands::SuperviseServer {
            server_bin,
            bind,
            control_bind,
            credentials_stdin,
        } => {
            crate::model_supervisor::run(root, &server_bin, &bind, &control_bind, credentials_stdin)
                .await
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn command_bundle_export(
    evidence_path: &Path,
    output: &Path,
    repository: &str,
    workflow: &str,
    workflow_run_id: &str,
    environment: &str,
    key_id: &str,
    signing_key_env: &str,
) -> Result<()> {
    use ha_eval_spec::app::{
        validate_evidence_bundle_manifest, EvidenceBundleManifest, EVIDENCE_BUNDLE_SCHEMA_VERSION,
    };
    use ring::signature::KeyPair as _;
    use zip::write::SimpleFileOptions;

    let evidence_raw = fs::read(evidence_path)
        .with_context(|| format!("reading model evidence {}", evidence_path.display()))?;
    let evidence_value: Value = serde_json::from_slice(&evidence_raw)?;
    scan_evidence_for_secrets(&evidence_value)?;
    let evidence: ModelCampaignEvidence = serde_json::from_value(evidence_value)?;
    validate_evidence_shape(&evidence)?;
    verify_evidence_artifact_files(evidence_path, &evidence.artifacts)?;
    if evidence.dirty || !evidence.source.is_release_eligible() {
        bail!("only clean protected-runner evidence can be signed");
    }
    if environment != "model-eval" {
        bail!("protected evidence bundles must use the model-eval environment identity");
    }
    let evidence_entry = "evidence/eval-model-campaign.v1.json".to_string();
    let manifest = EvidenceBundleManifest {
        schema_version: EVIDENCE_BUNDLE_SCHEMA_VERSION.to_string(),
        repository: repository.to_string(),
        workflow: workflow.to_string(),
        workflow_run_id: workflow_run_id.to_string(),
        environment: environment.to_string(),
        commit_sha: evidence.commit_sha.clone(),
        tier: evidence.tier,
        created_at: Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
        key_id: key_id.to_string(),
        evidence: ArtifactDigest {
            path: evidence_entry.clone(),
            sha256: sha256_bytes(&evidence_raw),
        },
        artifacts: evidence.artifacts.clone(),
    };
    validate_evidence_bundle_manifest(&manifest)?;
    let manifest_value = serde_json::to_value(&manifest)?;
    let manifest_canonical = ha_eval_spec::canonical_json(&manifest_value)?;
    let private_key_b64 = std::env::var(signing_key_env)
        .with_context(|| format!("missing protected signing key env {signing_key_env}"))?;
    let private_key = base64::engine::general_purpose::STANDARD
        .decode(private_key_b64.trim())
        .context("decoding Ed25519 PKCS#8 signing key")?;
    let key_pair = Ed25519KeyPair::from_pkcs8(&private_key)
        .map_err(|_| anyhow!("invalid Ed25519 PKCS#8 signing key"))?;
    let signature = base64::engine::general_purpose::STANDARD
        .encode(key_pair.sign(&manifest_canonical).as_ref());

    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut cursor);
        let options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .unix_permissions(0o600);
        zip.start_file("manifest.json", options)?;
        std::io::Write::write_all(&mut zip, &serde_json::to_vec_pretty(&manifest)?)?;
        zip.start_file("manifest.sig", options)?;
        std::io::Write::write_all(&mut zip, signature.as_bytes())?;
        zip.start_file(&evidence_entry, options)?;
        std::io::Write::write_all(&mut zip, &evidence_raw)?;
        let root = evidence_path.parent().unwrap_or_else(|| Path::new("."));
        for artifact in &evidence.artifacts {
            let path = root.join(&artifact.path);
            zip.start_file(&artifact.path, options)?;
            std::io::Write::write_all(&mut zip, &fs::read(path)?)?;
        }
        zip.finish()?;
    }
    ha_core::platform::write_atomic(output, cursor.get_ref())?;
    println!(
        "signed protected evidence bundle {} with key {} ({})",
        output.display(),
        key_id,
        key_pair.public_key().as_ref().len()
    );
    Ok(())
}

fn live_root(root: &Path) -> PathBuf {
    root.join("evals/live")
}

fn policy_path(root: &Path, tier: ModelCampaignTier) -> PathBuf {
    live_root(root)
        .join("policy")
        .join(format!("{}.json", tier.as_str()))
}

fn suite_path(root: &Path, id: &str) -> PathBuf {
    live_root(root).join("suites").join(id).join("suite.json")
}

fn load_policy(root: &Path, tier: ModelCampaignTier) -> Result<ModelCampaignPolicy> {
    let path = policy_path(root, tier);
    let raw: Value = read_json(&path)?;
    reject_embedded_secrets(&raw, "$")?;
    let policy: ModelCampaignPolicy = serde_json::from_value(raw)?;
    validate_model_policy(&policy)?;
    if policy.tier != tier {
        bail!("model policy {} tier does not match file name", policy.id);
    }
    Ok(policy)
}

fn load_suite(root: &Path, id: &str) -> Result<ModelCampaignSuite> {
    let path = suite_path(root, id);
    let raw: Value = read_json(&path)?;
    reject_embedded_secrets(&raw, "$")?;
    let suite: ModelCampaignSuite = serde_json::from_value(raw)?;
    validate_model_suite(&suite, &live_root(root))?;
    if suite.id != id {
        bail!(
            "model suite directory {id} contains manifest for {}",
            suite.id
        );
    }
    Ok(suite)
}

fn load_scenario(root: &Path, relative: &str) -> Result<(PathBuf, LiveAgentScenario)> {
    let path = ha_eval_spec::resolve_contained(&live_root(root), relative)?;
    let raw = read_json_or_yaml(&path)?;
    reject_embedded_secrets(&raw, "$")?;
    let scenario: LiveAgentScenario = serde_json::from_value(raw)?;
    let scenario_dir = path
        .parent()
        .ok_or_else(|| anyhow!("scenario path has no parent"))?;
    validate_scenario(&scenario, scenario_dir)?;
    Ok((path, scenario))
}

fn validate_repo(root: &Path) -> Result<()> {
    let live = live_root(root);
    let schemas = schema_paths(&live);
    let mut loaded_schemas = BTreeMap::new();
    for (name, path) in schemas {
        let schema: Value = read_json(&path)?;
        if schema.get("$schema").and_then(Value::as_str).is_none() {
            bail!("model {name} JSON Schema does not declare $schema");
        }
        loaded_schemas.insert(name, schema);
    }

    let mut policy_suite_ids = BTreeSet::new();
    let mut policy_digests = BTreeMap::new();
    for tier in [
        ModelCampaignTier::Nightly,
        ModelCampaignTier::Weekly,
        ModelCampaignTier::Release,
        ModelCampaignTier::Monthly,
    ] {
        let path = policy_path(root, tier);
        let raw: Value = read_json(&path)?;
        validate_json_schema(&raw, &loaded_schemas["policy"]).with_context(|| {
            format!("validating {} against model policy schema", path.display())
        })?;
        let policy = load_policy(root, tier)?;
        policy_digests.insert(
            format!("{}@{}", policy.id, policy.version),
            digest_serializable(&policy)?,
        );
        policy_suite_ids.extend(policy.suites.iter().map(|suite| suite.id.clone()));
    }

    let mut app_profile_digests = BTreeMap::new();
    let app_profiles_dir = live.join("app-profiles");
    let mut app_profile_paths = fs::read_dir(&app_profiles_dir)
        .with_context(|| format!("reading {}", app_profiles_dir.display()))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
        .collect::<Vec<_>>();
    app_profile_paths.sort();
    for path in app_profile_paths {
        let raw: Value = read_json(&path)?;
        validate_json_schema(&raw, &loaded_schemas["app_profile"])
            .with_context(|| format!("validating {} against app profile schema", path.display()))?;
        let profile: EvalAppProfile = read_json(&path)?;
        validate_app_profile(&profile)?;
        let digest = app_profile_digest(&profile)?;
        app_profile_digests.insert(format!("{}@{}", profile.id, profile.version), digest);
    }

    let mut trust_registry_digests = BTreeMap::new();
    let trust_registry_path = live.join("trust/evidence-keys.json");
    if trust_registry_path.exists() {
        let raw: Value = read_json(&trust_registry_path)?;
        validate_json_schema(&raw, &loaded_schemas["evidence_trust"]).with_context(|| {
            format!(
                "validating {} against evidence trust schema",
                trust_registry_path.display()
            )
        })?;
        let registry: EvidenceTrustRegistry = serde_json::from_value(raw)?;
        validate_trust_registry(&registry)?;
        let versioned_id = format!("evidence-keys@{}", registry.version);
        let digest = digest_serializable(&registry)?;
        println!("validated evidence trust registry {versioned_id}: sha256:{digest}");
        trust_registry_digests.insert(versioned_id, digest);
    }

    let mut suite_digests = BTreeMap::new();
    let mut scenario_digests = BTreeMap::new();
    let mut validated = Vec::new();
    for id in policy_suite_ids {
        let path = suite_path(root, &id);
        let raw: Value = read_json(&path)?;
        validate_json_schema(&raw, &loaded_schemas["suite"])
            .with_context(|| format!("validating {} against model suite schema", path.display()))?;
        let suite = load_suite(root, &id)?;
        let digest = model_suite_digest(&suite, &live)?;
        suite_digests.insert(format!("{}@{}", suite.id, suite.version), digest.clone());
        for case in &suite.cases {
            let (scenario_path, scenario) = load_scenario(root, &case.scenario_path)?;
            let raw = read_json_or_yaml(&scenario_path)?;
            validate_json_schema(&raw, &loaded_schemas["scenario"]).with_context(|| {
                format!(
                    "validating {} against live scenario schema",
                    scenario_path.display()
                )
            })?;
            let scenario_dir = scenario_path
                .parent()
                .ok_or_else(|| anyhow!("scenario path has no parent"))?;
            let digest = scenario_digest(&scenario, scenario_dir)?;
            let versioned = format!("{}@{}", scenario.id, scenario.version);
            if let Some(existing) = scenario_digests.insert(versioned.clone(), digest.clone()) {
                if existing != digest {
                    bail!("scenario {versioned} resolves to multiple content digests");
                }
            }
        }
        validated.push((suite.id, suite.cases.len(), digest));
    }
    validate_version_lock(
        &live.join("version-lock.json"),
        &suite_digests,
        &policy_digests,
        &scenario_digests,
        &app_profile_digests,
        &trust_registry_digests,
    )?;
    validated.sort_by(|a, b| a.0.cmp(&b.0));
    for (id, cases, digest) in validated {
        println!("validated live model suite {id}: {cases} cases, sha256:{digest}");
    }
    println!("validated real-model policies, scenarios, locks, and JSON Schemas");
    Ok(())
}

fn schema_paths(live: &Path) -> Vec<(&'static str, PathBuf)> {
    vec![
        (
            "scenario",
            live.join("schema/live-agent-scenario-v1.schema.json"),
        ),
        (
            "suite",
            live.join("schema/model-campaign-suite-v1.schema.json"),
        ),
        (
            "policy",
            live.join("schema/model-campaign-policy-v1.schema.json"),
        ),
        (
            "plan",
            live.join("schema/model-campaign-plan-v1.schema.json"),
        ),
        (
            "trial",
            live.join("schema/model-campaign-trial-v1.schema.json"),
        ),
        (
            "shard",
            live.join("schema/model-campaign-shard-v1.schema.json"),
        ),
        (
            "evidence",
            live.join("schema/eval-model-campaign-v1.schema.json"),
        ),
        (
            "waiver",
            live.join("schema/model-campaign-waiver-v1.schema.json"),
        ),
        (
            "app_profile",
            live.join("schema/eval-app-profile-v1.schema.json"),
        ),
        (
            "app_request",
            live.join("schema/eval-app-run-request-v1.schema.json"),
        ),
        ("app_plan", live.join("schema/eval-app-plan-v1.schema.json")),
        (
            "evidence_bundle",
            live.join("schema/eval-evidence-bundle-v1.schema.json"),
        ),
        (
            "evidence_trust",
            live.join("schema/eval-evidence-trust-v1.schema.json"),
        ),
    ]
}

fn validate_output_schema(root: &Path, name: &str, value: &impl serde::Serialize) -> Result<()> {
    let schema_path = schema_paths(&live_root(root))
        .into_iter()
        .find_map(|(candidate, path)| (candidate == name).then_some(path))
        .ok_or_else(|| anyhow!("unknown model output schema {name}"))?;
    let schema: Value = read_json(&schema_path)?;
    let raw = serde_json::to_value(value)?;
    validate_json_schema(&raw, &schema)
        .with_context(|| format!("validating generated model {name} output"))
}

fn validate_version_lock(
    path: &Path,
    suites: &BTreeMap<String, String>,
    policies: &BTreeMap<String, String>,
    scenarios: &BTreeMap<String, String>,
    app_profiles: &BTreeMap<String, String>,
    trust_registries: &BTreeMap<String, String>,
) -> Result<()> {
    let lock: Value = read_json(path)?;
    if lock.get("schemaVersion").and_then(Value::as_str) != Some("model-campaign-version-lock.v1") {
        bail!("unsupported model campaign version lock schema");
    }
    for (section, expected) in [
        ("suites", suites),
        ("policies", policies),
        ("scenarios", scenarios),
        ("appProfiles", app_profiles),
        ("trustRegistries", trust_registries),
    ] {
        let actual = lock
            .get(section)
            .and_then(Value::as_object)
            .ok_or_else(|| anyhow!("model version lock is missing {section}"))?;
        for (versioned_id, digest) in actual {
            let digest = digest
                .as_str()
                .ok_or_else(|| anyhow!("model version lock {versioned_id} is not a string"))?;
            validate_sha256(digest, &format!("model version lock {versioned_id}"))?;
        }
        for (versioned_id, digest) in expected {
            let locked = actual
                .get(versioned_id)
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("model version lock has no entry for {versioned_id}"))?;
            if locked != digest {
                bail!(
                    "{versioned_id} content changed without a version bump; restore it or append a new version-lock entry"
                );
            }
        }
    }
    Ok(())
}

fn command_plan(
    root: &Path,
    tier: ModelCampaignTier,
    reference: &str,
    output: Option<&Path>,
    format: ModelPlanFormat,
) -> Result<()> {
    validate_git_reference(reference)?;
    let plan = build_plan(root, tier, reference)?;
    validate_output_schema(root, "plan", &plan)?;
    if let Some(path) = output {
        write_json(path, &plan)?;
    }
    match format {
        ModelPlanFormat::Json => {
            if output.is_none() {
                println!("{}", serde_json::to_string_pretty(&plan)?);
            }
        }
        ModelPlanFormat::Github => {
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
                            "runnerClass": suite.runner_class,
                            "networkPolicy": suite.network_policy,
                        })
                    })
                })
                .collect::<Vec<_>>();
            println!("{}", serde_json::json!({"include": include}));
        }
    }
    Ok(())
}

fn build_plan(root: &Path, tier: ModelCampaignTier, reference: &str) -> Result<ModelCampaignPlan> {
    let policy = load_policy(root, tier)?;
    let application_version = app_version(root)?;
    build_plan_with_policy(root, reference, application_version, policy)
}

fn build_plan_with_policy(
    root: &Path,
    reference: &str,
    application_version: String,
    policy: ModelCampaignPolicy,
) -> Result<ModelCampaignPlan> {
    let tier = policy.tier;
    let live = live_root(root);
    let models = policy
        .models
        .iter()
        .map(|model| (model.role.as_str(), model))
        .collect::<BTreeMap<_, _>>();
    let allowed_adapters = policy
        .allowed_adapters
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let allowed_runners = policy
        .allowed_runner_classes
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let allowed_network = policy
        .allowed_network_policies
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let allowed_modes = policy
        .allowed_execution_modes
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let mut suites = Vec::new();
    let mut trials = Vec::new();
    for selected in &policy.suites {
        let suite = load_suite(root, &selected.id)?;
        validate_suite_policy_compatibility(
            &policy,
            &suite,
            selected,
            &allowed_adapters,
            &allowed_runners,
            &allowed_network,
            &allowed_modes,
        )?;
        let suite_digest = model_suite_digest(&suite, &live)?;
        let mut cases = Vec::new();
        for case in &suite.cases {
            if !selected.case_tags.is_empty()
                && !case.tags.iter().any(|tag| selected.case_tags.contains(tag))
            {
                continue;
            }
            let (scenario_path, scenario) = load_scenario(root, &case.scenario_path)?;
            if scenario.id != case.id {
                bail!(
                    "model case {} must reference a scenario with the same stable id, found {}",
                    case.id,
                    scenario.id
                );
            }
            validate_scenario_policy_compatibility(&policy, &suite, case, &scenario, &models)?;
            let scenario_dir = scenario_path
                .parent()
                .ok_or_else(|| anyhow!("scenario path has no parent"))?;
            let scenario_digest = scenario_digest(&scenario, scenario_dir)?;
            let (environment_digest, verifier_digest, prompt_digest, tool_schema_digest) =
                scenario_component_digests(&scenario, scenario_dir)?;
            let case_digest = model_case_digest(case, &scenario, scenario_dir)?;
            let repetitions = selected
                .repetitions
                .or(case.repetitions)
                .unwrap_or_else(|| suite.repetitions.for_tier(tier));
            if !(1..=20).contains(&repetitions) {
                bail!("model case {} repetitions must be 1..=20", case.id);
            }
            let roles = if case.model_roles.is_empty() {
                policy
                    .models
                    .iter()
                    .map(|model| model.role.clone())
                    .collect::<Vec<_>>()
            } else {
                case.model_roles.clone()
            };
            let arms = case.arms.clone();
            let experiment_arms = validate_experiment_arms(case, &scenario)?;
            let budget = strictest_budget(&[&policy.budget, &suite.budget, &scenario.budgets]);
            let timeout_seconds = [
                Some(suite.timeout_seconds),
                case.timeout_seconds,
                scenario.budgets.max_wall_seconds,
                policy.budget.max_wall_seconds,
            ]
            .into_iter()
            .flatten()
            .min()
            .unwrap_or(suite.timeout_seconds);
            let planned_case = PlannedModelCase {
                id: case.id.clone(),
                digest: case_digest.clone(),
                scenario_path: case.scenario_path.clone(),
                scenario_id: scenario.id.clone(),
                scenario_version: scenario.version.clone(),
                scenario_digest: scenario_digest.clone(),
                environment_digest: environment_digest.clone(),
                verifier_digest: verifier_digest.clone(),
                prompt_digest: prompt_digest.clone(),
                tool_schema_digest: tool_schema_digest.clone(),
                timeout_seconds,
                repetitions,
                budget,
                model_roles: roles.clone(),
                arms: arms.clone(),
                driver: scenario.subject.driver,
                tags: case.tags.clone(),
            };
            for role in roles {
                let model = models.get(role.as_str()).ok_or_else(|| {
                    anyhow!("model case {} references unknown role {role}", case.id)
                })?;
                let model_digest = digest_model_profile(model)?;
                for (arm, fault_profile, orchestration_profile) in &experiment_arms {
                    for trial_index in 0..repetitions {
                        trials.push(PlannedModelTrial {
                            id: stable_trial_id(
                                reference,
                                &suite.id,
                                &case.id,
                                &role,
                                arm,
                                trial_index,
                            ),
                            campaign_id: String::new(),
                            plan_digest: String::new(),
                            commit_sha: reference.to_string(),
                            app_version: application_version.clone(),
                            suite_id: suite.id.clone(),
                            suite_version: suite.version.clone(),
                            suite_digest: suite_digest.clone(),
                            case_id: case.id.clone(),
                            case_digest: case_digest.clone(),
                            trial_index,
                            seed: stable_trial_seed(
                                reference,
                                &suite.id,
                                &case.id,
                                &role,
                                arm,
                                trial_index,
                            ),
                            model_role: role.clone(),
                            arm: arm.clone(),
                            fault_profile: *fault_profile,
                            orchestration_profile: orchestration_profile.clone(),
                            model_digest: model_digest.clone(),
                            scenario_digest: scenario_digest.clone(),
                            environment_digest: environment_digest.clone(),
                            verifier_digest: verifier_digest.clone(),
                            prompt_digest: prompt_digest.clone(),
                            tool_schema_digest: tool_schema_digest.clone(),
                        });
                    }
                }
            }
            cases.push(planned_case);
        }
        if cases.is_empty() {
            bail!(
                "model policy selection leaves suite {} with no cases",
                suite.id
            );
        }
        suites.push(PlannedModelSuite {
            id: suite.id,
            version: suite.version,
            capability: suite.capability,
            adapter: suite.adapter,
            digest: suite_digest,
            runner_class: suite.runner_class,
            network_policy: suite.network_policy,
            execution_mode: suite.execution_mode,
            shards: suite.shards,
            cases,
        });
    }
    trials.sort_by(|left, right| left.id.cmp(&right.id));
    let policy_digest = digest_serializable(&policy)?;
    let mut plan = ModelCampaignPlan {
        schema_version: PLAN_SCHEMA_VERSION.to_string(),
        campaign_id: String::new(),
        plan_digest: String::new(),
        reference: reference.to_string(),
        tier,
        policy_id: policy.id,
        policy_version: policy.version,
        policy_digest,
        runner_digest: model_runner_digest(reference),
        app_version: application_version,
        campaign_budget: policy.campaign_budget,
        models: policy.models,
        suites,
        trials,
    };
    plan.plan_digest = immutable_plan_digest(&plan)?;
    plan.campaign_id = campaign_id_from_digest(&plan.plan_digest);
    for trial in &mut plan.trials {
        trial.plan_digest.clone_from(&plan.plan_digest);
        trial.campaign_id.clone_from(&plan.campaign_id);
    }
    Ok(plan)
}

pub(crate) fn list_app_profiles(root: &Path) -> Result<Vec<EvalAppProfile>> {
    let directory = live_root(root).join("app-profiles");
    let mut paths = fs::read_dir(&directory)
        .with_context(|| format!("reading {}", directory.display()))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
        .collect::<Vec<_>>();
    paths.sort();
    let schema: Value = read_json(&live_root(root).join("schema/eval-app-profile-v1.schema.json"))?;
    let mut profiles = Vec::with_capacity(paths.len());
    for path in paths {
        let raw: Value = read_json(&path)?;
        validate_json_schema(&raw, &schema)
            .with_context(|| format!("validating app profile {}", path.display()))?;
        let profile: EvalAppProfile = serde_json::from_value(raw)?;
        validate_app_profile(&profile)?;
        profiles.push(profile);
    }
    Ok(profiles)
}

pub(crate) fn list_app_catalog(root: &Path) -> Result<Vec<AppEvalSuiteCatalog>> {
    let suite_ids = list_app_profiles(root)?
        .into_iter()
        .flat_map(|profile| profile.suites.into_iter().map(|suite| suite.suite_id))
        .collect::<BTreeSet<_>>();
    let mut catalog = Vec::with_capacity(suite_ids.len());
    for suite_id in suite_ids {
        let suite = load_suite(root, &suite_id)?;
        let mut cases = Vec::with_capacity(suite.cases.len());
        for case in &suite.cases {
            let (_, scenario) = load_scenario(root, &case.scenario_path)?;
            cases.push(AppEvalCaseCatalog {
                id: case.id.clone(),
                title: scenario.title,
                tags: case.tags.clone(),
                arms: case.arms.clone(),
                timeout_seconds: case
                    .timeout_seconds
                    .unwrap_or(suite.timeout_seconds)
                    .min(suite.timeout_seconds),
            });
        }
        catalog.push(AppEvalSuiteCatalog {
            id: suite.id,
            version: suite.version,
            capability: suite.capability,
            cases,
        });
    }
    Ok(catalog)
}

fn load_app_profile(root: &Path, id: &str) -> Result<EvalAppProfile> {
    if id.is_empty()
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        bail!("app profile id is invalid");
    }
    let path = live_root(root)
        .join("app-profiles")
        .join(format!("{id}.json"));
    let profile: EvalAppProfile = read_json(&path)?;
    validate_app_profile(&profile)?;
    if profile.id != id {
        bail!("app profile filename and id differ");
    }
    Ok(profile)
}

pub(crate) fn build_app_plan(
    root: &Path,
    request: &EvalAppRunRequest,
    resolved_models: &[AppResolvedModelBinding],
    reference: &str,
    dirty: bool,
    application_version: &str,
    mut runtime_environment: RuntimeEnvironmentSnapshot,
) -> Result<EvalAppPlan> {
    validate_app_request(request)?;
    let profile = load_app_profile(root, &request.profile_id)?;
    if request.models.len() != resolved_models.len()
        || resolved_models.is_empty()
        || resolved_models.len() > usize::from(profile.max_models)
    {
        bail!("resolved App models do not match the request/profile");
    }
    for (selection, binding) in request.models.iter().zip(resolved_models) {
        if selection.provider_id != binding.model.provider_id
            || selection.model_id != binding.model.model_id
            || selection
                .reasoning_effort
                .as_ref()
                .is_some_and(|value| Some(value) != binding.model.reasoning_effort.as_ref())
            || selection
                .max_output_tokens
                .is_some_and(|value| Some(value) != binding.model.max_output_tokens)
        {
            bail!("resolved App model binding does not match its request selection");
        }
    }
    validate_app_request_against_profile(request, &profile)?;
    let asset_root_digest = digest_tree(&live_root(root))?;
    if !runtime_environment.asset_root_digest.is_empty()
        && runtime_environment.asset_root_digest != asset_root_digest
    {
        bail!("App runtime asset root does not match the Sidecar asset root");
    }
    runtime_environment
        .asset_root_digest
        .clone_from(&asset_root_digest);

    let child_budget = partition_budget(&request.campaign_budget, resolved_models.len())?;
    let mut campaigns = Vec::with_capacity(resolved_models.len());
    for binding in resolved_models {
        validate_sha256(
            &binding.provider_config_digest,
            "App Provider config digest",
        )?;
        validate_sha256(
            &binding.credential_config_digest,
            "App credential config digest",
        )?;
        let mut model = binding.model.clone();
        model.role = "anchor".to_string();
        ha_eval_spec::model::validate_model_profile(&model)?;
        let policy = app_policy(root, request, &profile, model.clone(), child_budget.clone())?;
        let mut child =
            build_plan_with_policy(root, reference, application_version.to_string(), policy)?;
        filter_app_child_plan(&mut child, request, &profile)?;
        child.plan_digest = immutable_plan_digest(&child)?;
        child.campaign_id = campaign_id_from_digest(&child.plan_digest);
        for trial in &mut child.trials {
            trial.plan_digest.clone_from(&child.plan_digest);
            trial.campaign_id.clone_from(&child.campaign_id);
        }
        campaigns.push(AppResolvedCampaign {
            campaign_id: child.campaign_id.clone(),
            plan_digest: child.plan_digest.clone(),
            model: model.clone(),
            model_digest: digest_model_profile(&model)?,
            provider_config_digest: binding.provider_config_digest.clone(),
            credential_config_digest: binding.credential_config_digest.clone(),
            resolved_plan: child,
        });
    }

    let total_trials = campaigns
        .iter()
        .map(|campaign| campaign.resolved_plan.trials.len())
        .sum::<usize>();
    if total_trials == 0
        || total_trials > usize::from(profile.max_trials)
        || total_trials > APP_MAX_TRIALS
    {
        bail!("App plan contains {total_trials} trials, exceeding profile/global safety limits");
    }
    apply_app_trial_timeout_limits(
        &mut campaigns,
        profile.max_trial_seconds,
        request.campaign_budget.max_wall_seconds,
        total_trials,
    )?;

    let profile_digest = app_profile_digest(&profile)?;
    let selection_digest = digest_serializable(&request.redacted())?;
    let mut plan = EvalAppPlan {
        schema_version: APP_PLAN_SCHEMA_VERSION.to_string(),
        experiment_id: String::new(),
        plan_digest: String::new(),
        reference: reference.to_string(),
        dirty,
        app_version: application_version.to_string(),
        source: ModelCampaignSource::LocalApp,
        execution_profile: AppExecutionProfile::LocalNativeDiagnostic,
        profile_id: profile.id,
        profile_version: profile.version,
        profile_digest,
        selection_digest,
        asset_root_digest,
        runtime_environment,
        debug_retention: request.debug_retention,
        campaign_budget: request.campaign_budget.clone(),
        campaigns,
    };
    plan.plan_digest = immutable_app_plan_digest(&plan)?;
    plan.experiment_id = format!("exp-{}", &plan.plan_digest[..20]);
    validate_app_plan(&plan)?;
    validate_output_schema(root, "app_plan", &plan)?;
    Ok(plan)
}

fn validate_app_request_against_profile(
    request: &EvalAppRunRequest,
    profile: &EvalAppProfile,
) -> Result<()> {
    if request
        .campaign_budget
        .max_cost_usd
        .is_some_and(|value| value > profile.max_cost_usd)
        || request
            .campaign_budget
            .max_concurrency
            .is_some_and(|value| value > u32::from(profile.max_concurrency))
    {
        bail!("App request exceeds selected profile budget");
    }
    let profile_suites = profile
        .suites
        .iter()
        .map(|suite| suite.suite_id.as_str())
        .collect::<BTreeSet<_>>();
    if !request.suite_selections.is_empty() && !profile.allow_custom {
        bail!("App profile {} does not allow custom selection", profile.id);
    }
    for selection in &request.suite_selections {
        if !profile_suites.contains(selection.suite_id.as_str()) {
            bail!(
                "App request suite {} is not allowed by profile {}",
                selection.suite_id,
                profile.id
            );
        }
        if profile.use_suite_repetitions && selection.repetitions.is_some() {
            bail!(
                "App profile {} fixes repetitions in its suite manifest",
                profile.id
            );
        }
        for arm in &selection.arms {
            if !profile.allowed_arms.contains(arm) {
                bail!(
                    "App request arm {arm} is not allowed by profile {}",
                    profile.id
                );
            }
        }
    }
    Ok(())
}

fn app_policy(
    root: &Path,
    request: &EvalAppRunRequest,
    profile: &EvalAppProfile,
    model: ModelProfile,
    child_budget: CampaignBudget,
) -> Result<ModelCampaignPolicy> {
    let request_suites = request
        .suite_selections
        .iter()
        .map(|selection| (selection.suite_id.as_str(), selection))
        .collect::<BTreeMap<_, _>>();
    let selected_profiles = if request.suite_selections.is_empty() {
        profile.suites.iter().collect::<Vec<_>>()
    } else {
        profile
            .suites
            .iter()
            .filter(|suite| request_suites.contains_key(suite.suite_id.as_str()))
            .collect::<Vec<_>>()
    };
    let mut allowed_adapters = BTreeSet::new();
    let mut allowed_runners = BTreeSet::new();
    let mut allowed_network = BTreeSet::new();
    let mut allowed_modes = BTreeSet::new();
    let mut suites = Vec::new();
    for selected in selected_profiles {
        let suite = load_suite(root, &selected.suite_id)?;
        allowed_adapters.insert(suite.adapter);
        allowed_runners.insert(suite.runner_class);
        allowed_network.insert(suite.network_policy);
        allowed_modes.insert(suite.execution_mode);
        let requested = request_suites.get(selected.suite_id.as_str()).copied();
        suites.push(ModelPolicySuite {
            id: selected.suite_id.clone(),
            required: false,
            case_tags: selected.case_tags.clone(),
            repetitions: requested
                .and_then(|selection| selection.repetitions)
                .or(profile.default_repetitions)
                .filter(|_| !profile.use_suite_repetitions),
        });
    }
    let policy = ModelCampaignPolicy {
        schema_version: POLICY_SCHEMA_VERSION.to_string(),
        id: format!("app-{}", profile.id),
        version: profile.version.clone(),
        tier: profile.base_tier,
        mode: PolicyMode::Advisory,
        allowed_adapters: allowed_adapters.into_iter().collect(),
        allowed_runner_classes: allowed_runners.into_iter().collect(),
        allowed_network_policies: allowed_network.into_iter().collect(),
        allowed_execution_modes: allowed_modes.into_iter().collect(),
        allowed_sources: vec![ModelCampaignSource::LocalApp],
        models: vec![model],
        suites,
        // App campaign limits are aggregate user-spend/run-scheduling limits,
        // not per-trial compute limits. Registered suite/scenario budgets stay
        // authoritative for each trial; feeding maxConcurrency=1 here would
        // incorrectly fail ordinary title/extraction attribution and would
        // conflate "one concurrent trial" with "one active task span".
        budget: CampaignBudget::default(),
        campaign_budget: child_budget,
        allow_llm_judge: false,
        performance_blocking: false,
        require_model_snapshot: false,
        artifact_retention_days: 30,
        max_infra_error_rate: 1.0,
    };
    validate_model_policy(&policy)?;
    Ok(policy)
}

fn filter_app_child_plan(
    plan: &mut ModelCampaignPlan,
    request: &EvalAppRunRequest,
    profile: &EvalAppProfile,
) -> Result<()> {
    let requested = request
        .suite_selections
        .iter()
        .map(|selection| (selection.suite_id.as_str(), selection))
        .collect::<BTreeMap<_, _>>();
    for suite in &mut plan.suites {
        let suite_request = requested.get(suite.id.as_str()).copied();
        if let Some(selection) = suite_request {
            let available = suite
                .cases
                .iter()
                .map(|case| case.id.as_str())
                .collect::<BTreeSet<_>>();
            for id in &selection.case_ids {
                if !available.contains(id.as_str()) {
                    bail!("App request references unknown/unselected case {id}");
                }
            }
        }
        for case in &mut suite.cases {
            if suite_request.is_some_and(|selection| {
                !selection.case_ids.is_empty() && !selection.case_ids.contains(&case.id)
            }) {
                case.arms.clear();
                continue;
            }
            let requested_arms = suite_request
                .map(|selection| selection.arms.as_slice())
                .unwrap_or_default();
            let allowed = case
                .arms
                .iter()
                .filter(|arm| profile.allowed_arms.contains(arm))
                .filter(|arm| requested_arms.is_empty() || requested_arms.contains(arm))
                .cloned()
                .collect::<Vec<_>>();
            case.arms = match profile.arm_mode {
                AppArmMode::AllAllowed => allowed,
                AppArmMode::OneControlPerCase => {
                    preferred_control_arm(&allowed).into_iter().collect()
                }
            };
            if !profile.use_suite_repetitions {
                if let Some(repetitions) = suite_request.and_then(|selection| selection.repetitions)
                {
                    case.repetitions = repetitions;
                }
            }
        }
        suite.cases.retain(|case| !case.arms.is_empty());
    }
    plan.suites.retain(|suite| !suite.cases.is_empty());
    if plan.suites.is_empty() {
        bail!("App request leaves no runnable suites/cases");
    }
    let selected = plan
        .suites
        .iter()
        .flat_map(|suite| {
            suite.cases.iter().flat_map(move |case| {
                case.arms.iter().map(move |arm| {
                    (
                        suite.id.as_str(),
                        case.id.as_str(),
                        arm.as_str(),
                        case.repetitions,
                    )
                })
            })
        })
        .collect::<Vec<_>>();
    plan.trials.retain(|trial| {
        selected.iter().any(|(suite, case, arm, repetitions)| {
            trial.suite_id == *suite
                && trial.case_id == *case
                && trial.arm == *arm
                && trial.trial_index < *repetitions
        })
    });
    if plan.trials.is_empty() {
        bail!("App request leaves no runnable trials");
    }
    plan.trials.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(())
}

fn preferred_control_arm(arms: &[String]) -> Option<String> {
    [
        "team_full_control",
        "control",
        "single_agent_compute_matched_control",
    ]
    .into_iter()
    .find_map(|preferred| arms.iter().find(|arm| arm.as_str() == preferred).cloned())
    .or_else(|| arms.iter().find(|arm| arm.ends_with("_control")).cloned())
}

/// Keep one slow Provider call from consuming the entire desktop experiment.
/// The App wall budget is an experiment-wide ceiling, so every immutable child
/// case receives at most its profile cap and a conservative fair share of the
/// total wall clock. Ten percent stays reserved for process startup, evidence
/// writes, cancellation, and Supervisor shutdown.
fn apply_app_trial_timeout_limits(
    campaigns: &mut [AppResolvedCampaign],
    profile_max_trial_seconds: Option<u64>,
    experiment_max_wall_seconds: Option<u64>,
    total_trials: usize,
) -> Result<()> {
    let fair_share = experiment_max_wall_seconds.map(|seconds| {
        seconds
            .saturating_mul(9)
            .saturating_div(10)
            .saturating_div(total_trials.max(1) as u64)
            .max(1)
    });
    let ceiling = match (profile_max_trial_seconds, fair_share) {
        (Some(profile), Some(fair)) => Some(profile.min(fair)),
        (Some(profile), None) => Some(profile),
        (None, Some(fair)) => Some(fair),
        (None, None) => None,
    };
    let Some(ceiling) = ceiling else {
        return Ok(());
    };

    for campaign in campaigns {
        for suite in &mut campaign.resolved_plan.suites {
            for case in &mut suite.cases {
                case.timeout_seconds = case.timeout_seconds.min(ceiling);
                case.budget.max_wall_seconds = Some(
                    case.budget
                        .max_wall_seconds
                        .map_or(ceiling, |registered| registered.min(ceiling)),
                );
            }
        }
        campaign.resolved_plan.plan_digest = immutable_plan_digest(&campaign.resolved_plan)?;
        campaign.resolved_plan.campaign_id =
            campaign_id_from_digest(&campaign.resolved_plan.plan_digest);
        for trial in &mut campaign.resolved_plan.trials {
            trial
                .plan_digest
                .clone_from(&campaign.resolved_plan.plan_digest);
            trial
                .campaign_id
                .clone_from(&campaign.resolved_plan.campaign_id);
        }
        campaign
            .plan_digest
            .clone_from(&campaign.resolved_plan.plan_digest);
        campaign
            .campaign_id
            .clone_from(&campaign.resolved_plan.campaign_id);
    }
    Ok(())
}

fn partition_budget(budget: &CampaignBudget, parts: usize) -> Result<CampaignBudget> {
    let parts_u64 = parts.max(1) as u64;
    let divide = |dimension: &str, value: Option<u64>| -> Result<Option<u64>> {
        value
            .map(|value| {
                if value < parts_u64 {
                    bail!(
                        "App {dimension} budget must be at least the selected model count ({parts})"
                    );
                }
                Ok(value / parts_u64)
            })
            .transpose()
    };
    Ok(CampaignBudget {
        max_wall_seconds: budget.max_wall_seconds,
        max_model_calls: divide("model-call", budget.max_model_calls)?,
        max_input_tokens: divide("input-token", budget.max_input_tokens)?,
        max_output_tokens: divide("output-token", budget.max_output_tokens)?,
        max_cost_usd: budget.max_cost_usd.map(|value| value / parts.max(1) as f64),
        max_tool_calls: divide("tool-call", budget.max_tool_calls)?,
        max_agents: budget.max_agents,
        max_concurrency: budget.max_concurrency,
    })
}

fn digest_tree(root: &Path) -> Result<String> {
    fn visit(base: &Path, current: &Path, files: &mut BTreeMap<String, String>) -> Result<()> {
        let mut entries = fs::read_dir(current)
            .with_context(|| format!("reading asset directory {}", current.display()))?
            .collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() {
                bail!(
                    "App asset root may not contain symlinks: {}",
                    path.display()
                );
            }
            if metadata.is_dir() {
                visit(base, &path, files)?;
            } else if metadata.is_file() {
                let relative = path
                    .strip_prefix(base)?
                    .to_string_lossy()
                    .replace('\\', "/");
                files.insert(relative, digest_file(&path)?);
            }
        }
        Ok(())
    }
    let mut files = BTreeMap::new();
    visit(root, root, &mut files)?;
    digest_serializable(&files)
}

pub(crate) fn live_asset_root_digest(root: &Path) -> Result<String> {
    digest_tree(&live_root(root))
}

fn immutable_app_plan_digest(plan: &EvalAppPlan) -> Result<String> {
    let mut unsigned = plan.clone();
    unsigned.experiment_id.clear();
    unsigned.plan_digest.clear();
    digest_serializable(&unsigned)
}

fn validate_experiment_arms(
    case: &ModelCampaignCaseSpec,
    scenario: &LiveAgentScenario,
) -> Result<Vec<(String, FaultProfile, Option<String>)>> {
    let has_faults = !scenario.faults.is_empty();
    let mut parsed = Vec::with_capacity(case.arms.len());
    if let Some(comparison) = &scenario.comparison {
        let expected_profiles = comparison
            .ablations
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        for arm in &case.arms {
            let (profile, fault_profile) = if let Some(profile) = arm.strip_suffix("_control") {
                (profile, FaultProfile::Clean)
            } else if let Some(profile) = arm.strip_suffix("_faulted") {
                (profile, FaultProfile::Chaos)
            } else {
                bail!(
                    "comparison case {} arm {arm} must end in _control or _faulted",
                    case.id
                );
            };
            if !expected_profiles.contains(profile) {
                bail!(
                    "comparison case {} arm {arm} references unknown profile {profile}",
                    case.id
                );
            }
            parsed.push((arm.clone(), fault_profile, Some(profile.to_string())));
        }
        for profile in expected_profiles {
            if !parsed.iter().any(|(_, fault, orchestration)| {
                *fault == FaultProfile::Clean && orchestration.as_deref() == Some(profile)
            }) {
                bail!(
                    "comparison case {} is missing clean arm for {profile}",
                    case.id
                );
            }
            let has_chaos = parsed.iter().any(|(_, fault, orchestration)| {
                *fault == FaultProfile::Chaos && orchestration.as_deref() == Some(profile)
            });
            if has_faults != has_chaos {
                bail!(
                    "comparison case {} must declare chaos arm for {profile} exactly when faults exist",
                    case.id
                );
            }
        }
    } else {
        for arm in &case.arms {
            let fault_profile = match arm.as_str() {
                "control" => FaultProfile::Clean,
                "faulted" => FaultProfile::Chaos,
                _ => bail!(
                    "non-comparison case {} only permits control/faulted arms",
                    case.id
                ),
            };
            parsed.push((arm.clone(), fault_profile, None));
        }
        let has_chaos = parsed
            .iter()
            .any(|(_, fault, _)| *fault == FaultProfile::Chaos);
        if has_faults != has_chaos {
            bail!(
                "model case {} must declare faulted arm exactly when its scenario declares faults",
                case.id
            );
        }
    }
    Ok(parsed)
}

fn immutable_plan_digest(plan: &ModelCampaignPlan) -> Result<String> {
    let mut material = plan.clone();
    material.campaign_id.clear();
    material.plan_digest.clear();
    for trial in &mut material.trials {
        trial.campaign_id.clear();
        trial.plan_digest.clear();
    }
    digest_serializable(&material)
}

fn campaign_id_from_digest(plan_digest: &str) -> String {
    format!("mcampaign_{}", &plan_digest[..plan_digest.len().min(24)])
}

fn validate_plan(root: &Path, plan: &ModelCampaignPlan) -> Result<()> {
    if plan.schema_version != PLAN_SCHEMA_VERSION {
        bail!("unsupported model plan schemaVersion");
    }
    validate_git_reference(&plan.reference)?;
    validate_sha256(&plan.plan_digest, "model plan digest")?;
    if immutable_plan_digest(plan)? != plan.plan_digest
        || campaign_id_from_digest(&plan.plan_digest) != plan.campaign_id
    {
        bail!("model plan digest/campaign identity is invalid");
    }
    let expected = build_plan(root, plan.tier, &plan.reference)?;
    if &expected != plan {
        bail!("model plan is stale or does not match current policy/assets");
    }
    Ok(())
}

/// Validate a plan at an execution boundary. Release/CLI plans continue to be
/// rebuilt from the checked-in policy. A desktop plan is accepted only when a
/// validated App wrapper is explicitly attached to the process tree.
fn validate_execution_plan(root: &Path, plan: &ModelCampaignPlan) -> Result<Option<EvalAppPlan>> {
    if std::env::var(APP_CONTROL_ENV).as_deref() != Ok("1") {
        validate_plan(root, plan)?;
        return Ok(None);
    }
    let app_plan_path = std::env::var_os(APP_PLAN_ENV)
        .map(PathBuf::from)
        .ok_or_else(|| {
            anyhow!("App-controlled model evaluation is missing its immutable App plan")
        })?;
    let app_plan: EvalAppPlan = read_json(&app_plan_path)?;
    validate_local_child_plan(root, &app_plan, plan)?;
    Ok(Some(app_plan))
}

fn validate_local_child_plan(
    root: &Path,
    app_plan: &EvalAppPlan,
    child: &ModelCampaignPlan,
) -> Result<()> {
    validate_app_plan(app_plan)?;
    validate_git_reference(&app_plan.reference)?;
    if immutable_app_plan_digest(app_plan)? != app_plan.plan_digest
        || app_plan.experiment_id != format!("exp-{}", &app_plan.plan_digest[..20])
    {
        bail!("App plan digest/experiment identity is invalid");
    }
    let profile = load_app_profile(root, &app_plan.profile_id)?;
    if profile.version != app_plan.profile_version
        || app_profile_digest(&profile)? != app_plan.profile_digest
    {
        bail!("App plan profile is stale or does not match current assets");
    }
    let asset_root_digest = digest_tree(&live_root(root))?;
    if asset_root_digest != app_plan.asset_root_digest
        || asset_root_digest != app_plan.runtime_environment.asset_root_digest
    {
        bail!("App plan asset root is stale or does not match the runner");
    }
    let wrapper = app_plan
        .campaigns
        .iter()
        .find(|campaign| campaign.campaign_id == child.campaign_id)
        .ok_or_else(|| anyhow!("model child campaign is not registered by the App plan"))?;
    if &wrapper.resolved_plan != child
        || wrapper.plan_digest != child.plan_digest
        || wrapper.model_digest != digest_model_profile(&wrapper.model)?
        || child.reference != app_plan.reference
        || child.app_version != app_plan.app_version
        || child.models.as_slice() != [wrapper.model.clone()]
        || immutable_plan_digest(child)? != child.plan_digest
        || campaign_id_from_digest(&child.plan_digest) != child.campaign_id
    {
        bail!("App child plan identity or model binding is invalid");
    }
    validate_local_child_assets(root, child)?;
    Ok(())
}

fn validate_local_child_assets(root: &Path, plan: &ModelCampaignPlan) -> Result<()> {
    if plan.schema_version != PLAN_SCHEMA_VERSION || plan.tier == ModelCampaignTier::Release {
        bail!("App child plan uses an unsupported schema or release tier");
    }
    if plan.models.len() != 1 || plan.models[0].role != "anchor" {
        bail!("App child plan must bind exactly one anchor model");
    }
    validate_sha256(&plan.policy_digest, "App child policy digest")?;
    validate_sha256(&plan.runner_digest, "App child runner digest")?;
    let model_digest = digest_model_profile(&plan.models[0])?;
    let live = live_root(root);
    let mut suite_ids = BTreeSet::new();
    let mut expected_trials = BTreeMap::<String, PlannedModelTrial>::new();
    for planned_suite in &plan.suites {
        if !suite_ids.insert(planned_suite.id.as_str()) {
            bail!("App child plan contains a duplicate suite");
        }
        let suite = load_suite(root, &planned_suite.id)?;
        if planned_suite.version != suite.version
            || planned_suite.capability != suite.capability
            || planned_suite.adapter != suite.adapter
            || planned_suite.digest != model_suite_digest(&suite, &live)?
            || planned_suite.runner_class != suite.runner_class
            || planned_suite.network_policy != suite.network_policy
            || planned_suite.execution_mode != suite.execution_mode
            || planned_suite.shards != suite.shards
            || planned_suite.network_policy == NetworkPolicy::LiveWeb
        {
            bail!(
                "App child suite {} differs from registered local assets",
                suite.id
            );
        }
        let mut case_ids = BTreeSet::new();
        for planned_case in &planned_suite.cases {
            if !case_ids.insert(planned_case.id.as_str()) {
                bail!("App child suite {} contains a duplicate case", suite.id);
            }
            let case = suite
                .cases
                .iter()
                .find(|case| case.id == planned_case.id)
                .ok_or_else(|| {
                    anyhow!("App child plan references unknown case {}", planned_case.id)
                })?;
            let (scenario_path, scenario) = load_scenario(root, &case.scenario_path)?;
            let scenario_dir = scenario_path
                .parent()
                .ok_or_else(|| anyhow!("scenario path has no parent"))?;
            let scenario_hash = scenario_digest(&scenario, scenario_dir)?;
            let (environment_digest, verifier_digest, prompt_digest, tool_schema_digest) =
                scenario_component_digests(&scenario, scenario_dir)?;
            let registered_arms = validate_experiment_arms(case, &scenario)?;
            let registered_arm_names = registered_arms
                .iter()
                .map(|(arm, _, _)| arm.as_str())
                .collect::<BTreeSet<_>>();
            if planned_case.scenario_path != case.scenario_path
                || planned_case.scenario_id != scenario.id
                || planned_case.scenario_version != scenario.version
                || planned_case.digest != model_case_digest(case, &scenario, scenario_dir)?
                || planned_case.scenario_digest != scenario_hash
                || planned_case.environment_digest != environment_digest
                || planned_case.verifier_digest != verifier_digest
                || planned_case.prompt_digest != prompt_digest
                || planned_case.tool_schema_digest != tool_schema_digest
                || planned_case.driver != scenario.subject.driver
                || planned_case.tags != case.tags
                || planned_case.model_roles != ["anchor".to_string()]
                || !(1..=5).contains(&planned_case.repetitions)
                || planned_case.arms.is_empty()
                || planned_case
                    .arms
                    .iter()
                    .any(|arm| !registered_arm_names.contains(arm.as_str()))
            {
                bail!(
                    "App child case {} differs from registered local assets",
                    case.id
                );
            }
            let registered_budget = strictest_budget(&[&suite.budget, &scenario.budgets]);
            if !budget_is_narrower_or_equal(&planned_case.budget, &registered_budget) {
                bail!("App child case {} expands its registered budget", case.id);
            }
            if planned_case.timeout_seconds > suite.timeout_seconds
                || scenario
                    .budgets
                    .max_wall_seconds
                    .is_some_and(|limit| planned_case.timeout_seconds > limit)
            {
                bail!("App child case {} expands its timeout", case.id);
            }
            for arm in &planned_case.arms {
                let (_, fault_profile, orchestration_profile) = registered_arms
                    .iter()
                    .find(|(registered, _, _)| registered == arm)
                    .ok_or_else(|| anyhow!("App child case has an unregistered arm"))?;
                for trial_index in 0..planned_case.repetitions {
                    let id = stable_trial_id(
                        &plan.reference,
                        &suite.id,
                        &case.id,
                        "anchor",
                        arm,
                        trial_index,
                    );
                    expected_trials.insert(
                        id.clone(),
                        PlannedModelTrial {
                            id,
                            campaign_id: plan.campaign_id.clone(),
                            plan_digest: plan.plan_digest.clone(),
                            commit_sha: plan.reference.clone(),
                            app_version: plan.app_version.clone(),
                            suite_id: suite.id.clone(),
                            suite_version: suite.version.clone(),
                            suite_digest: planned_suite.digest.clone(),
                            case_id: case.id.clone(),
                            case_digest: planned_case.digest.clone(),
                            trial_index,
                            seed: stable_trial_seed(
                                &plan.reference,
                                &suite.id,
                                &case.id,
                                "anchor",
                                arm,
                                trial_index,
                            ),
                            model_role: "anchor".to_string(),
                            arm: arm.clone(),
                            fault_profile: *fault_profile,
                            orchestration_profile: orchestration_profile.clone(),
                            model_digest: model_digest.clone(),
                            scenario_digest: scenario_hash.clone(),
                            environment_digest: environment_digest.clone(),
                            verifier_digest: verifier_digest.clone(),
                            prompt_digest: prompt_digest.clone(),
                            tool_schema_digest: tool_schema_digest.clone(),
                        },
                    );
                }
            }
        }
    }
    let actual_trials = plan
        .trials
        .iter()
        .map(|trial| (trial.id.clone(), trial.clone()))
        .collect::<BTreeMap<_, _>>();
    if actual_trials != expected_trials {
        bail!("App child trials do not exactly match the narrowed registered cases");
    }
    Ok(())
}

fn budget_is_narrower_or_equal(actual: &CampaignBudget, ceiling: &CampaignBudget) -> bool {
    fn within_u64(actual: Option<u64>, ceiling: Option<u64>) -> bool {
        match (actual, ceiling) {
            (_, None) => true,
            (Some(actual), Some(ceiling)) => actual <= ceiling,
            (None, Some(_)) => false,
        }
    }
    fn within_u32(actual: Option<u32>, ceiling: Option<u32>) -> bool {
        match (actual, ceiling) {
            (_, None) => true,
            (Some(actual), Some(ceiling)) => actual <= ceiling,
            (None, Some(_)) => false,
        }
    }
    within_u64(actual.max_wall_seconds, ceiling.max_wall_seconds)
        && within_u64(actual.max_model_calls, ceiling.max_model_calls)
        && within_u64(actual.max_input_tokens, ceiling.max_input_tokens)
        && within_u64(actual.max_output_tokens, ceiling.max_output_tokens)
        && within_u64(actual.max_tool_calls, ceiling.max_tool_calls)
        && within_u32(actual.max_agents, ceiling.max_agents)
        && within_u32(actual.max_concurrency, ceiling.max_concurrency)
        && match (actual.max_cost_usd, ceiling.max_cost_usd) {
            (_, None) => true,
            (Some(actual), Some(ceiling)) => actual <= ceiling,
            (None, Some(_)) => false,
        }
}

fn parse_model_shard(value: &str) -> Result<(u16, u16)> {
    let (index, total) = value
        .split_once('/')
        .ok_or_else(|| anyhow!("shard must use i/n form"))?;
    let index = index.parse::<u16>().context("parsing model shard index")?;
    let total = total.parse::<u16>().context("parsing model shard total")?;
    if total == 0 || index == 0 || index > total {
        bail!("model shard must satisfy 1 <= i <= n");
    }
    Ok((index - 1, total))
}

async fn command_smoke(root: &Path, server_bin: Option<&Path>, output: &Path) -> Result<()> {
    validate_repo(root)?;
    let reference = exact_head_sha(root)?;
    let plan = build_plan(root, ModelCampaignTier::Nightly, &reference)?;
    let trial = plan
        .trials
        .iter()
        .find(|trial| {
            trial.case_id == "HA-GL-001" && trial.model_role == "anchor" && trial.arm == "control"
        })
        .cloned()
        .ok_or_else(|| anyhow!("nightly plan has no HA-GL-001 anchor/control smoke trial"))?;
    let suite = plan
        .suites
        .iter()
        .find(|suite| suite.id == trial.suite_id)
        .ok_or_else(|| anyhow!("fake Provider smoke suite is missing from plan"))?;
    let planned_case = suite
        .cases
        .iter()
        .find(|case| case.id == trial.case_id)
        .cloned()
        .ok_or_else(|| anyhow!("fake Provider smoke case is missing from plan"))?;
    let model = plan
        .models
        .iter()
        .find(|model| model.role == trial.model_role)
        .cloned()
        .ok_or_else(|| anyhow!("fake Provider smoke model is missing from plan"))?;
    if model.provider_id != "eval-anchor" || model.model_id != "configured-anchor-v1" {
        bail!("fake Provider smoke plan must use the registered anchor model identity");
    }
    let (scenario_path, scenario) = load_scenario(root, &planned_case.scenario_path)?;
    let smoke_result_content = fake_smoke_result_content(&scenario_path, &scenario)?;

    let fake_provider = crate::model_fake::FakeProvider::start(smoke_result_content).await?;
    let isolated = tempfile::tempdir().context("creating fake Provider smoke directory")?;
    let data_dir = isolated.path().join("hope-data");
    let trial_home = isolated.path().join("trial-home");
    fs::create_dir_all(&trial_home)?;
    crate::model_fake::write_smoke_config(&data_dir, &fake_provider.base_url)?;
    let runtime_config_digest = digest_file(&data_dir.join("config.json"))?;
    let server_bin = server_bin
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::model_fake::default_server_bin(root));
    let mut server = crate::model_fake::spawn_hope_server(&server_bin, &data_dir)?;
    crate::model_fake::wait_healthy(&mut server).await?;

    let scoped_env = ScopedEnv::set(&[
        ("HA_MODEL_EVAL_SERVER_URL", server.base_url.as_str()),
        (
            "HA_MODEL_EVAL_SERVER_TOKEN",
            crate::model_fake::FAKE_SERVER_TOKEN,
        ),
        ("HA_MODEL_EVAL_SOURCE", "local_cli"),
        (RUNTIME_CONFIG_DIGEST_ENV, runtime_config_digest.as_str()),
        (
            "HA_MODEL_EVAL_TRIAL_HOME",
            trial_home
                .to_str()
                .ok_or_else(|| anyhow!("fake Provider trial path is not UTF-8"))?,
        ),
    ]);
    let result = crate::model_adapter::run_registered_trial(
        root,
        suite.adapter,
        &scenario_path,
        &scenario,
        &planned_case,
        &trial,
        &model,
        1,
    )
    .await;
    drop(scoped_env);
    drop(server);
    fake_provider.shutdown().await;

    validate_trial_result(&trial, &result)?;
    validate_output_schema(root, "trial", &result)?;
    write_json(output, &result)?;
    validate_model_trace(
        &result.trace,
        &result.trace_events,
        "fake Provider smoke trial",
    )?;
    if result.outcome != ModelCampaignOutcome::Passed {
        bail!(
            "fake Provider smoke did not pass (outcome={:?}, class={:?}, error={:?}); inspect {}",
            result.outcome,
            result.failure_class,
            result.error,
            output.display()
        );
    }
    if result.attribution != AttributionCompleteness::Complete
        || !result.trace.closed
        || result.orchestration.model_calls < 2
        || result.tools.attempted < 6
        || result.tokens.input.is_none_or(|tokens| tokens == 0)
        || result.tokens.output.is_none_or(|tokens| tokens == 0)
    {
        bail!(
            "fake Provider smoke passed business verifiers but telemetry coverage is incomplete; inspect {}",
            output.display()
        );
    }
    println!(
        "fake Provider smoke passed with {} model calls, {} tool attempts, and a closed {}-span trace",
        result.orchestration.model_calls, result.tools.attempted, result.trace.span_count
    );
    Ok(())
}

async fn command_app_control_smoke(
    root: &Path,
    sidecar: Option<&Path>,
    server_bin: Option<&Path>,
) -> Result<()> {
    validate_repo(root)?;
    let reference = exact_head_sha(root)?;
    let base_plan = build_plan(root, ModelCampaignTier::Nightly, &reference)?;
    let smoke_trial = base_plan
        .trials
        .iter()
        .find(|trial| {
            trial.case_id == "HA-GL-001" && trial.model_role == "anchor" && trial.arm == "control"
        })
        .ok_or_else(|| anyhow!("nightly plan has no HA-GL-001 anchor/control smoke trial"))?;
    let smoke_suite = base_plan
        .suites
        .iter()
        .find(|suite| suite.id == smoke_trial.suite_id)
        .ok_or_else(|| anyhow!("App-control smoke suite is missing from plan"))?;
    let smoke_case = smoke_suite
        .cases
        .iter()
        .find(|case| case.id == smoke_trial.case_id)
        .ok_or_else(|| anyhow!("App-control smoke case is missing from plan"))?;
    let (scenario_path, scenario) = load_scenario(root, &smoke_case.scenario_path)?;
    let result_content = fake_smoke_result_content(&scenario_path, &scenario)?;
    let fake_provider = crate::model_fake::FakeProvider::start(result_content).await?;

    let sidecar = sidecar
        .map(PathBuf::from)
        .unwrap_or(std::env::current_exe()?)
        .canonicalize()?;
    let server_bin = server_bin
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::model_fake::default_server_bin(root))
        .canonicalize()
        .with_context(|| "canonicalizing App-control smoke Hope server")?;
    let product_binary_digest = digest_file(&server_bin)?;
    let config = crate::model_fake::smoke_config(&fake_provider.base_url);
    let provider = config
        .providers
        .iter()
        .find(|provider| provider.id == crate::model_fake::PROVIDER_ID)
        .ok_or_else(|| anyhow!("fake Provider config has no registered Provider"))?;
    let model_config = provider
        .models
        .iter()
        .find(|model| model.id == crate::model_fake::MODEL_ID)
        .ok_or_else(|| anyhow!("fake Provider config has no registered model"))?;
    let model = ModelProfile {
        role: "anchor".to_string(),
        provider_id: provider.id.clone(),
        model_id: model_config.id.clone(),
        snapshot: Some(model_config.id.clone()),
        temperature: config.temperature,
        reasoning_effort: (!config.reasoning_effort.trim().is_empty())
            .then(|| config.reasoning_effort.clone()),
        max_output_tokens: Some(u64::from(model_config.max_tokens)),
    };
    let resolved_models = vec![AppResolvedModelBinding {
        model,
        provider_config_digest: digest_serializable(provider)?,
        credential_config_digest: digest_serializable(&serde_json::json!({
            "kind": "fake_provider_smoke",
            "providerId": provider.id,
        }))?,
    }];
    let request = EvalAppRunRequest {
        schema_version: APP_REQUEST_SCHEMA_VERSION.to_string(),
        profile_id: "custom".to_string(),
        suite_selections: vec![AppSuiteRequest {
            suite_id: smoke_suite.id.clone(),
            case_ids: vec![smoke_case.id.clone()],
            arms: vec!["control".to_string()],
            // Two repetitions land on different stable shards for this fixture.  Running them
            // together proves that the App concurrency control schedules real trial work rather
            // than merely accepting and persisting `maxConcurrency`.
            repetitions: Some(2),
        }],
        models: vec![AppModelSelection {
            provider_id: provider.id.clone(),
            model_id: model_config.id.clone(),
            credential_profile_ref: None,
            reasoning_effort: None,
            max_output_tokens: None,
        }],
        campaign_budget: CampaignBudget {
            max_wall_seconds: Some(300),
            max_model_calls: Some(100),
            max_input_tokens: Some(1_000_000),
            max_output_tokens: Some(200_000),
            max_cost_usd: Some(5.0),
            max_tool_calls: Some(200),
            max_agents: Some(16),
            max_concurrency: Some(2),
        },
        debug_retention: AppDebugRetention::Redacted,
        consent: AppEvalConsent {
            model_costs: true,
            synthetic_tool_execution: true,
        },
    };
    let runtime_environment = RuntimeEnvironmentSnapshot {
        actual_runner_class: "local_native".to_string(),
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        os_version: None,
        network_enforcement: NetworkEnforcement::Unverified,
        product_binary_digest: product_binary_digest.clone(),
        runner_binary_digest: digest_file(&sidecar)?,
        asset_root_digest: live_asset_root_digest(root)?,
        hardware_class_digest: None,
        container_image_digest: None,
    };
    let isolated = tempfile::tempdir().context("creating App-control smoke directory")?;
    let output_root = isolated.path().join("outputs");
    fs::create_dir_all(&output_root)?;
    let provider_secrets_b64 = crate::model_fake::provider_secrets_b64()?;

    let mut sidecar_command = tokio::process::Command::new(&sidecar);
    sidecar_command
        .arg("--root")
        .arg(root)
        .arg("model")
        .arg("app-control")
        .env(
            crate::model_app_control::PARENT_PID_ENV,
            std::process::id().to_string(),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);
    #[cfg(unix)]
    sidecar_command.process_group(0);
    let mut child = sidecar_command
        .spawn()
        .context("starting App-control smoke Sidecar")?;
    let mut input = child.stdin.take().context("Sidecar smoke stdin missing")?;
    let output = child
        .stdout
        .take()
        .context("Sidecar smoke stdout missing")?;
    let mut output = BufReader::new(output);
    let result = async {
        let mut event_seq = 0u64;
        let hello_event =
            read_app_smoke_event_with_timeout(&mut output, &mut event_seq, Duration::from_secs(8))
                .await?;
        let AppControlEvent::Hello { hello } = hello_event else {
            bail!("App-control smoke did not receive hello first");
        };
        if hello.protocol_version != APP_CONTROL_PROTOCOL_VERSION
            || hello.product_version != env!("CARGO_PKG_VERSION")
            || hello.runner_digest != digest_file(&sidecar)?
            || hello.asset_root_digest != runtime_environment.asset_root_digest
        {
            bail!("App-control smoke Sidecar identity mismatch");
        }
        let mut command_seq = 1u64;
        write_app_smoke_command(
            &mut input,
            command_seq,
            &AppControlCommand::HelloAck {
                protocol_version: hello.protocol_version.clone(),
                product_version: hello.product_version.clone(),
                asset_root_digest: hello.asset_root_digest.clone(),
            },
        )
        .await?;
        if !matches!(
            read_app_smoke_event_with_timeout(&mut output, &mut event_seq, Duration::from_secs(8))
                .await?,
            AppControlEvent::Ready
        ) {
            bail!("App-control smoke handshake was rejected");
        }
        command_seq += 1;
        write_app_smoke_command(&mut input, command_seq, &AppControlCommand::ListProfiles).await?;
        let AppControlEvent::Profiles { profiles } =
            read_app_smoke_event_with_timeout(&mut output, &mut event_seq, Duration::from_secs(30))
                .await?
        else {
            bail!("App-control smoke profile response missing");
        };
        if profiles.is_empty() {
            bail!("App-control smoke returned no profiles");
        }
        for profile in &profiles {
            validate_app_profile(profile)?;
        }
        command_seq += 1;
        write_app_smoke_command(&mut input, command_seq, &AppControlCommand::ListCatalog).await?;
        let AppControlEvent::Catalog { suites } =
            read_app_smoke_event_with_timeout(&mut output, &mut event_seq, Duration::from_secs(30))
                .await?
        else {
            bail!("App-control smoke catalog response missing");
        };
        if suites.is_empty() || hello.adapters.iter().any(|adapter| adapter == "bfcl_v4") {
            bail!("App-control smoke catalog/adapters are inconsistent with installed Harnesses");
        }

        command_seq += 1;
        write_app_smoke_command(
            &mut input,
            command_seq,
            &AppControlCommand::Preview {
                request: request.clone(),
                resolved_models: resolved_models.clone(),
                reference: reference.clone(),
                dirty: git_dirty(root),
                app_version: env!("CARGO_PKG_VERSION").to_string(),
                runtime_environment: runtime_environment.clone(),
            },
        )
        .await?;
        let AppControlEvent::Preview { plan } =
            read_app_smoke_event_with_timeout(&mut output, &mut event_seq, Duration::from_secs(30))
                .await?
        else {
            bail!("App-control smoke preview response missing");
        };
        validate_app_plan(&plan)?;
        if plan.campaigns.len() != 1
            || plan.campaigns[0].resolved_plan.trials.len() != 2
            || plan.campaigns[0]
                .resolved_plan
                .trials
                .iter()
                .any(|trial| trial.case_id != "HA-GL-001")
        {
            bail!("App-control smoke preview did not resolve the requested repetitions");
        }

        command_seq += 1;
        write_app_smoke_command(
            &mut input,
            command_seq,
            &AppControlCommand::Start {
                request: request.clone(),
                resolved_models: resolved_models.clone(),
                reference: reference.clone(),
                dirty: git_dirty(root),
                app_version: env!("CARGO_PKG_VERSION").to_string(),
                runtime_environment: runtime_environment.clone(),
                product_binary: server_bin.to_string_lossy().to_string(),
                product_binary_digest: product_binary_digest.clone(),
                output_root: output_root.to_string_lossy().to_string(),
                config: serde_json::to_value(&config)?,
                provider_secrets_b64: provider_secrets_b64.clone(),
            },
        )
        .await?;
        let start_event =
            read_app_smoke_event_with_timeout(&mut output, &mut event_seq, Duration::from_secs(30))
                .await?;
        let (experiment_id, plan_digest) = match start_event {
            AppControlEvent::Started {
                experiment_id,
                plan_digest,
            } => (experiment_id, plan_digest),
            AppControlEvent::Error { code, message, .. } => {
                bail!("App-control smoke start rejected ({code}): {message}")
            }
            _ => bail!("App-control smoke start acknowledgement missing"),
        };
        if experiment_id != plan.experiment_id || plan_digest != plan.plan_digest {
            bail!("App-control smoke preview/start identity changed");
        }

        let evidence_paths = loop {
            let event = read_app_smoke_event_with_timeout(
                &mut output,
                &mut event_seq,
                Duration::from_secs(330),
            )
            .await?;
            match event {
                AppControlEvent::Completed {
                    experiment_id: completed_id,
                    evidence_paths,
                } if completed_id == experiment_id => break evidence_paths,
                AppControlEvent::Error { code, message, .. } => {
                    bail!("App-control smoke failed ({code}): {message}")
                }
                AppControlEvent::Cancelled { .. } => {
                    bail!("App-control smoke was unexpectedly cancelled")
                }
                _ => {}
            }
        };
        if evidence_paths.len() != 1 {
            bail!("App-control smoke produced an unexpected evidence count");
        }
        let evidence_path = PathBuf::from(&evidence_paths[0]);
        let evidence: ModelCampaignEvidence = read_json(&evidence_path)?;
        validate_evidence_shape(&evidence)?;
        if evidence.source != ModelCampaignSource::LocalApp
            || evidence.execution_profile != Some(AppExecutionProfile::LocalNativeDiagnostic)
            || evidence.app_plan_digest.as_deref() != Some(plan.plan_digest.as_str())
            || evidence.trial_results.len() != 2
        {
            bail!("App-control smoke evidence identity is incomplete");
        }
        if let Some(smoke_result) = evidence
            .trial_results
            .iter()
            .find(|trial| trial.outcome != ModelCampaignOutcome::Passed)
        {
            bail!(
                "App-control smoke trial did not pass (outcome={:?}, class={:?}, error={:?}, failed checks={:?})",
                smoke_result.outcome,
                smoke_result.failure_class,
                smoke_result.error,
                smoke_result
                    .milestones
                    .iter()
                    .chain(smoke_result.invariants.iter())
                    .chain(smoke_result.judge_checks.iter())
                    .filter(|check| !check.passed)
                    .map(|check| (&check.id, &check.detail))
                    .collect::<Vec<_>>()
            );
        }
        if evidence.trial_results.iter().any(|trial| {
            trial.orchestration.model_calls < 2
                || trial.tools.attempted < 6
                || trial.tokens.input.unwrap_or(0) == 0
                || trial.tokens.output.unwrap_or(0) == 0
        })
        {
            bail!("App-control smoke evidence telemetry is incomplete");
        }
        let first = &evidence.trial_results[0];
        let second = &evidence.trial_results[1];
        let first_started = parse_timestamp(&first.started_at, "first App smoke trial startedAt")?;
        let first_completed =
            parse_timestamp(&first.completed_at, "first App smoke trial completedAt")?;
        let second_started =
            parse_timestamp(&second.started_at, "second App smoke trial startedAt")?;
        let second_completed =
            parse_timestamp(&second.completed_at, "second App smoke trial completedAt")?;
        if first_started >= second_completed || second_started >= first_completed {
            bail!("App-control smoke repetitions did not overlap at maxConcurrency=2");
        }
        let model_calls = evidence
            .trial_results
            .iter()
            .map(|trial| trial.orchestration.model_calls)
            .sum::<u64>();
        let tool_calls = evidence
            .trial_results
            .iter()
            .map(|trial| trial.tools.attempted)
            .sum::<u64>();
        let input_tokens = evidence
            .trial_results
            .iter()
            .map(|trial| trial.tokens.input.unwrap_or(0))
            .sum::<u64>();
        let output_tokens = evidence
            .trial_results
            .iter()
            .map(|trial| trial.tokens.output.unwrap_or(0))
            .sum::<u64>();
        let experiment_root = output_root.join(&experiment_id);
        if experiment_root.join("runtime").exists() {
            bail!("App-control smoke runtime secrets directory was not removed");
        }
        scan_app_smoke_output_for_secret(
            &experiment_root,
            &[
                crate::model_fake::FAKE_PROVIDER_KEY.as_bytes(),
                provider_secrets_b64.as_bytes(),
            ],
        )?;

        command_seq += 1;
        write_app_smoke_command(&mut input, command_seq, &AppControlCommand::Shutdown).await?;
        if !matches!(
            read_app_smoke_event_with_timeout(&mut output, &mut event_seq, Duration::from_secs(15))
                .await?,
            AppControlEvent::Bye
        ) {
            bail!("App-control smoke Sidecar did not shut down cleanly");
        }
        Ok::<_, anyhow::Error>((
            profiles.len(),
            suites.len(),
            model_calls,
            tool_calls,
            input_tokens,
            output_tokens,
        ))
    }
    .await;

    if result.is_err() {
        #[cfg(unix)]
        if let Some(id) = child.id() {
            unsafe {
                libc::kill(-(id as i32), libc::SIGKILL);
            }
        }
        let _ = child.kill().await;
    }
    let status = tokio::time::timeout(Duration::from_secs(15), child.wait()).await;
    if status.is_err() {
        #[cfg(unix)]
        if let Some(id) = child.id() {
            unsafe {
                libc::kill(-(id as i32), libc::SIGKILL);
            }
        }
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
    fake_provider.shutdown().await;
    let (profile_count, suite_count, model_calls, tool_calls, input_tokens, output_tokens) =
        result?;
    let status = status.context("App-control smoke Sidecar did not exit")??;
    if !status.success() {
        bail!("App-control smoke Sidecar exited with {status}");
    }
    println!(
        "App-control smoke passed end-to-end: {profile_count} profiles, {suite_count} suites, {} model calls, {} tool attempts, {} input + {} output tokens",
        model_calls,
        tool_calls,
        input_tokens,
        output_tokens,
    );
    Ok(())
}

fn scan_app_smoke_output_for_secret(root: &Path, secrets: &[&[u8]]) -> Result<()> {
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        for entry in fs::read_dir(&path)? {
            let entry = entry?;
            let metadata = fs::symlink_metadata(entry.path())?;
            if metadata.file_type().is_symlink() {
                bail!("App-control smoke output contains a symlink");
            }
            if metadata.is_dir() {
                pending.push(entry.path());
                continue;
            }
            if !metadata.is_file() {
                continue;
            }
            if metadata.len() > 256 * 1024 * 1024 {
                bail!("App-control smoke output artifact exceeds inspection limit");
            }
            let bytes = fs::read(entry.path())?;
            if secrets.iter().any(|secret| {
                !secret.is_empty() && bytes.windows(secret.len()).any(|part| part == *secret)
            }) {
                bail!(
                    "App-control smoke Provider secret leaked into {}",
                    entry.path().display()
                );
            }
        }
    }
    Ok(())
}

async fn write_app_smoke_command(
    input: &mut tokio::process::ChildStdin,
    seq: u64,
    command: &AppControlCommand,
) -> Result<()> {
    let envelope = AppControlEnvelope {
        protocol_version: APP_CONTROL_PROTOCOL_VERSION.to_string(),
        campaign_id: command.correlation_id().map(str::to_string),
        seq,
        timestamp: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        payload: command,
    };
    let mut bytes = serde_json::to_vec(&envelope)?;
    bytes.push(b'\n');
    input.write_all(&bytes).await?;
    input.flush().await?;
    Ok(())
}

async fn read_app_smoke_event(
    output: &mut BufReader<tokio::process::ChildStdout>,
    last_seq: &mut u64,
) -> Result<AppControlEvent> {
    let mut line = String::new();
    if output.read_line(&mut line).await? == 0 {
        bail!("App-control smoke protocol stream closed");
    }
    let envelope: AppControlEnvelope<AppControlEvent> = serde_json::from_str(&line)?;
    validate_app_control_envelope(&envelope)?;
    if envelope.campaign_id.as_deref() != envelope.payload.correlation_id() {
        bail!("App-control smoke event correlation identity mismatch");
    }
    DateTime::parse_from_rfc3339(&envelope.timestamp)?;
    if envelope.seq != last_seq.saturating_add(1) {
        bail!("App-control smoke event sequence gap");
    }
    *last_seq = envelope.seq;
    Ok(envelope.payload)
}

async fn read_app_smoke_event_with_timeout(
    output: &mut BufReader<tokio::process::ChildStdout>,
    last_seq: &mut u64,
    timeout: Duration,
) -> Result<AppControlEvent> {
    tokio::time::timeout(timeout, read_app_smoke_event(output, last_seq))
        .await
        .context("App-control smoke event timed out")?
}

fn fake_smoke_result_content(scenario_path: &Path, scenario: &LiveAgentScenario) -> Result<String> {
    let scenario_dir = scenario_path
        .parent()
        .ok_or_else(|| anyhow!("fake Provider scenario path has no parent"))?;
    let verifier = scenario
        .verifiers
        .iter()
        .find(|verifier| verifier.id == "business_artifact")
        .ok_or_else(|| anyhow!("fake Provider smoke scenario has no business artifact verifier"))?;
    let config_path = verifier
        .config_path
        .as_deref()
        .ok_or_else(|| anyhow!("fake Provider business artifact verifier has no config"))?;
    let config: Value = read_json(&resolve_contained(scenario_dir, config_path)?)?;
    let expected = config
        .get("expectedSubset")
        .ok_or_else(|| anyhow!("fake Provider artifact verifier has no expectedSubset"))?;
    let mut content = serde_json::to_string_pretty(expected)?;
    content.push('\n');
    Ok(content)
}

struct ScopedEnv(Vec<(String, Option<OsString>)>);

impl ScopedEnv {
    fn set(values: &[(&str, &str)]) -> Self {
        let previous = values
            .iter()
            .map(|(key, value)| {
                let previous = std::env::var_os(key);
                std::env::set_var(key, value);
                ((*key).to_string(), previous)
            })
            .collect();
        Self(previous)
    }
}

impl Drop for ScopedEnv {
    fn drop(&mut self) {
        for (key, value) in self.0.drain(..).rev() {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
}

fn exact_head_sha(root: &Path) -> Result<String> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(root)
        .output()
        .context("resolving current commit for fake Provider smoke")?;
    if !output.status.success() {
        bail!("git rev-parse HEAD failed for fake Provider smoke");
    }
    let reference = String::from_utf8(output.stdout)
        .context("git rev-parse HEAD returned non-UTF-8 output")?
        .trim()
        .to_string();
    validate_git_reference(&reference)?;
    Ok(reference)
}

pub(crate) fn current_runtime_config_digest() -> Option<String> {
    std::env::var(RUNTIME_CONFIG_DIGEST_ENV)
        .ok()
        .filter(|value| is_lower_sha256(value))
        .or_else(|| {
            let data_dir = std::env::var_os("HA_DATA_DIR")?;
            digest_file(&PathBuf::from(data_dir).join("config.json")).ok()
        })
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn command_run(
    root: &Path,
    plan_path: &Path,
    suite_id: &str,
    shard: &str,
    output: &Path,
    confirm_model_costs: bool,
) -> Result<()> {
    if !confirm_model_costs && std::env::var("HA_MODEL_EVAL_CONFIRM_COSTS").as_deref() != Ok("1") {
        bail!(
            "real-model evaluation may call Provider APIs and incur charges; rerun with --confirm-model-costs"
        );
    }
    let plan_path = plan_path
        .canonicalize()
        .with_context(|| format!("canonicalizing model plan {}", plan_path.display()))?;
    let plan: ModelCampaignPlan = read_json(&plan_path)?;
    validate_execution_plan(root, &plan)?;
    let suite = plan
        .suites
        .iter()
        .find(|suite| suite.id == suite_id)
        .ok_or_else(|| anyhow!("model suite {suite_id} is not in plan"))?;
    let (shard_index, shard_total) = parse_model_shard(shard)?;
    if shard_total != suite.shards {
        bail!(
            "model shard total {shard_total} does not match planned {}",
            suite.shards
        );
    }
    enforce_runtime_network_contract(suite.network_policy)?;
    let selected = plan
        .trials
        .iter()
        .filter(|trial| {
            trial.suite_id == suite.id && stable_shard(&trial.id, shard_total) == shard_index
        })
        .collect::<Vec<_>>();
    let shard_budget =
        scale_campaign_budget(&plan.campaign_budget, selected.len(), plan.trials.len());
    let started_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let mut results = Vec::with_capacity(selected.len());
    let mut stop_reason: Option<String> = None;
    for trial in selected {
        if let Some(reason) = &stop_reason {
            let now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
            results.push(failed_trial_result(
                trial,
                1,
                &now,
                &now,
                ModelCampaignOutcome::Cancelled,
                "campaign_budget_stop",
                reason,
                0,
            ));
            continue;
        }
        let planned_case = suite
            .cases
            .iter()
            .find(|case| case.id == trial.case_id)
            .ok_or_else(|| anyhow!("planned trial references a missing case"))?;
        let mut result = run_trial_subprocess(root, &plan_path, trial, planned_case, 1)?;
        if matches!(
            result.outcome,
            ModelCampaignOutcome::InfraError | ModelCampaignOutcome::SimulatorError
        ) {
            let first_attempt = result.as_attempt_record();
            result = run_trial_subprocess(root, &plan_path, trial, planned_case, 2)?;
            merge_retry_usage(&mut result, &first_attempt)?;
            result.prior_attempts.push(first_attempt);
        }
        validate_trial_result(trial, &result)?;
        results.push(result);
        stop_reason = campaign_budget_stop_reason(&shard_budget, &results, 0.90);
    }
    results.sort_by(|left, right| left.trial_id.cmp(&right.trial_id));
    let completed_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let duration_ms = wall_duration_ms(
        parse_timestamp(&started_at, "model shard startedAt")?,
        parse_timestamp(&completed_at, "model shard completedAt")?,
    )?;
    let trace_path = shard_trace_path(output)?;
    let trace_jsonl = render_trace_jsonl(&results)?;
    if let Some(parent) = trace_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&trace_path, trace_jsonl.as_bytes())
        .with_context(|| format!("writing redacted model trace {}", trace_path.display()))?;
    let trace_artifact = ArtifactDigest {
        path: trace_path
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| anyhow!("model trace output has no UTF-8 file name"))?
            .to_string(),
        sha256: digest_file(&trace_path)?,
    };
    let shard_result = ModelShardResult {
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
        trace_artifact,
        trials: results,
    };
    validate_output_schema(root, "shard", &shard_result)?;
    write_json(output, &shard_result)
}

fn shard_trace_path(output: &Path) -> Result<PathBuf> {
    let name = output
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| anyhow!("model shard output must have a UTF-8 file name"))?;
    Ok(output.with_file_name(format!("{name}.trace.jsonl")))
}

fn render_trace_jsonl(results: &[ModelTrialResult]) -> Result<String> {
    let mut output = String::new();
    for result in results {
        for prior in &result.prior_attempts {
            append_trace_attempt(
                &mut output,
                &result.trial_id,
                prior.attempt,
                &prior.trace.trace_id,
                &prior.trace_events,
            )?;
        }
        append_trace_attempt(
            &mut output,
            &result.trial_id,
            result.attempt,
            &result.trace.trace_id,
            &result.trace_events,
        )?;
    }
    Ok(output)
}

fn append_trace_attempt(
    output: &mut String,
    trial_id: &str,
    attempt: u8,
    trace_id: &str,
    events: &[ha_eval_spec::model::ModelCampaignEvent],
) -> Result<()> {
    for event in events {
        let value = serde_json::json!({
            "schemaVersion": "model-campaign-trace-event.v1",
            "trialId": trial_id,
            "attempt": attempt,
            "traceId": trace_id,
            "event": event,
        });
        scan_evidence_for_secrets(&value)?;
        output.push_str(&serde_json::to_string(&value)?);
        output.push('\n');
    }
    Ok(())
}

/// Make the top-level trial metrics cumulative across runner retry attempts.
/// The earlier attempt remains available verbatim in `priorAttempts`, while
/// budgets and successful-run efficiency see the real total cost of obtaining
/// the final outcome.
fn merge_retry_usage(
    result: &mut ModelTrialResult,
    prior: &ha_eval_spec::model::ModelTrialAttemptRecord,
) -> Result<()> {
    let prior_started = parse_timestamp(&prior.started_at, "prior attempt startedAt")?;
    let current_started = parse_timestamp(&result.started_at, "retry attempt startedAt")?;
    let completed = parse_timestamp(&result.completed_at, "retry attempt completedAt")?;
    let started = prior_started.min(current_started);
    result.started_at = started.to_rfc3339_opts(SecondsFormat::Millis, true);
    result.timings.wall_ms = wall_duration_ms(started, completed)?;
    result.timings.environment_setup_ms = result
        .timings
        .environment_setup_ms
        .saturating_add(prior.timings.environment_setup_ms);
    result.timings.environment_cleanup_ms = result
        .timings
        .environment_cleanup_ms
        .saturating_add(prior.timings.environment_cleanup_ms);
    result.timings.model_active_ms = result
        .timings
        .model_active_ms
        .saturating_add(prior.timings.model_active_ms);
    result.timings.tool_active_ms = result
        .timings
        .tool_active_ms
        .saturating_add(prior.timings.tool_active_ms);
    result.timings.queue_wait_ms = result
        .timings
        .queue_wait_ms
        .saturating_add(prior.timings.queue_wait_ms);
    result.timings.approval_wait_ms = result
        .timings
        .approval_wait_ms
        .saturating_add(prior.timings.approval_wait_ms);
    result.timings.environment_wait_ms = result
        .timings
        .environment_wait_ms
        .saturating_add(prior.timings.environment_wait_ms);
    result.timings.critical_path_ms = result.timings.wall_ms;
    result.timings.ttft_ms = match (prior.timings.ttft_ms, result.timings.ttft_ms) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    };

    // A subprocess/HTTP failure before any model call has exactly zero model
    // usage even though the fallback attempt record carries null fields. Do
    // not turn a fully priced retry into unknown cost in that case.
    if prior.orchestration.model_calls > 0 {
        merge_optional_u64(&mut result.tokens.input, prior.tokens.input);
        merge_optional_u64(&mut result.tokens.output, prior.tokens.output);
        merge_optional_u64(&mut result.tokens.cache_read, prior.tokens.cache_read);
        merge_optional_u64(&mut result.tokens.cache_write, prior.tokens.cache_write);
        merge_optional_u64(&mut result.tokens.reasoning, prior.tokens.reasoning);
        if result.tokens.usage_source != prior.tokens.usage_source {
            result.tokens.usage_source = Some("mixed_attempt_sources".to_string());
        }
        merge_optional_f64(&mut result.cost.total_usd, prior.cost.total_usd);
        merge_optional_f64(&mut result.cost.agent_usd, prior.cost.agent_usd);
        merge_optional_f64(&mut result.cost.simulator_usd, prior.cost.simulator_usd);
        merge_optional_f64(&mut result.cost.judge_usd, prior.cost.judge_usd);
        result.cost.price_snapshot_digest = merge_price_digests(
            prior.cost.price_snapshot_digest.as_deref(),
            result.cost.price_snapshot_digest.as_deref(),
        );
    }

    result.tools.attempted = result.tools.attempted.saturating_add(prior.tools.attempted);
    result.tools.logical_calls = result
        .tools
        .logical_calls
        .saturating_add(prior.tools.logical_calls);
    result.tools.succeeded = result.tools.succeeded.saturating_add(prior.tools.succeeded);
    result.tools.failed = result.tools.failed.saturating_add(prior.tools.failed);
    result.tools.cancelled = result.tools.cancelled.saturating_add(prior.tools.cancelled);
    result.tools.retries = result.tools.retries.saturating_add(prior.tools.retries);
    result.tools.parse_errors = result
        .tools
        .parse_errors
        .saturating_add(prior.tools.parse_errors);
    result.tools.invalid = result.tools.invalid.saturating_add(prior.tools.invalid);
    result.tools.duplicate = result.tools.duplicate.saturating_add(prior.tools.duplicate);
    result.tools.unused_results = result
        .tools
        .unused_results
        .saturating_add(prior.tools.unused_results);
    result.tools.effective = result.tools.effective.saturating_add(prior.tools.effective);

    result.orchestration.model_calls = result
        .orchestration
        .model_calls
        .saturating_add(prior.orchestration.model_calls);
    result.orchestration.model_retries = result
        .orchestration
        .model_retries
        .saturating_add(prior.orchestration.model_retries);
    result.orchestration.failovers = result
        .orchestration
        .failovers
        .saturating_add(prior.orchestration.failovers);
    result.orchestration.loop_iterations = result
        .orchestration
        .loop_iterations
        .saturating_add(prior.orchestration.loop_iterations);
    result.orchestration.replans = result
        .orchestration
        .replans
        .saturating_add(prior.orchestration.replans);
    result.orchestration.checkpoints = result
        .orchestration
        .checkpoints
        .saturating_add(prior.orchestration.checkpoints);
    result.orchestration.resumes = result
        .orchestration
        .resumes
        .saturating_add(prior.orchestration.resumes);
    result.orchestration.spawned_agents = result
        .orchestration
        .spawned_agents
        .saturating_add(prior.orchestration.spawned_agents);
    result.orchestration.max_agent_depth = result
        .orchestration
        .max_agent_depth
        .max(prior.orchestration.max_agent_depth);
    result.orchestration.max_concurrency = result
        .orchestration
        .max_concurrency
        .max(prior.orchestration.max_concurrency);
    result.orchestration.handoffs = result
        .orchestration
        .handoffs
        .saturating_add(prior.orchestration.handoffs);
    merge_optional_u64(
        &mut result.orchestration.coordination_tokens,
        prior.orchestration.coordination_tokens,
    );
    result.orchestration.child_active_ms = result
        .orchestration
        .child_active_ms
        .saturating_add(prior.orchestration.child_active_ms);
    result.orchestration.async_jobs = result
        .orchestration
        .async_jobs
        .saturating_add(prior.orchestration.async_jobs);
    result.orchestration.duplicate_injections = result
        .orchestration
        .duplicate_injections
        .saturating_add(prior.orchestration.duplicate_injections);
    result.orchestration.orphaned_children = result
        .orchestration
        .orphaned_children
        .saturating_add(prior.orchestration.orphaned_children);
    // `trace` and `traceEvents` describe the final/current attempt only.
    // The earlier graph is retained verbatim in `priorAttempts`; combining
    // only its counters here would create a summary that cannot be
    // recomputed from the current event stream and would fail strict causal
    // validation. Usage and wall metrics above remain cumulative so budgets
    // still see the real cost of recovery.
    result.warnings.push(format!(
        "runner retried after attempt {} ended as {:?}",
        prior.attempt, prior.outcome
    ));
    Ok(())
}

fn merge_optional_u64(target: &mut Option<u64>, previous: Option<u64>) {
    *target = match (*target, previous) {
        (Some(current), Some(previous)) => Some(current.saturating_add(previous)),
        _ => None,
    };
}

fn merge_optional_f64(target: &mut Option<f64>, previous: Option<f64>) {
    *target = match (*target, previous) {
        (Some(current), Some(previous)) => Some(current + previous),
        _ => None,
    };
}

fn merge_price_digests(previous: Option<&str>, current: Option<&str>) -> Option<String> {
    match (previous, current) {
        (Some(previous), Some(current)) if previous == current => Some(current.to_string()),
        (Some(previous), Some(current)) => {
            let mut values = [previous, current];
            values.sort_unstable();
            Some(sha256_bytes(
                format!("price-snapshots-v1:{}:{}", values[0], values[1]).as_bytes(),
            ))
        }
        _ => None,
    }
}

fn scale_campaign_budget(
    budget: &CampaignBudget,
    selected_trials: usize,
    all_trials: usize,
) -> CampaignBudget {
    if selected_trials == 0 || all_trials == 0 {
        return CampaignBudget::default();
    }
    let numerator = selected_trials as u128;
    let denominator = all_trials as u128;
    let scale_u64 = |value: Option<u64>| {
        value.map(|value| {
            ((u128::from(value) * numerator).div_ceil(denominator)).min(u128::from(u64::MAX)) as u64
        })
    };
    let ratio = selected_trials as f64 / all_trials as f64;
    CampaignBudget {
        // Shards run concurrently, so every shard receives the campaign's
        // elapsed-time ceiling. Token/call/cost ceilings are consumptive and
        // are apportioned by the shard's stable trial share.
        max_wall_seconds: budget.max_wall_seconds,
        max_model_calls: scale_u64(budget.max_model_calls),
        max_input_tokens: scale_u64(budget.max_input_tokens),
        max_output_tokens: scale_u64(budget.max_output_tokens),
        max_cost_usd: budget.max_cost_usd.map(|value| value * ratio),
        max_tool_calls: scale_u64(budget.max_tool_calls),
        // Agent/concurrency limits are instantaneous ceilings, not additive
        // campaign consumption, so do not divide them by shard size.
        max_agents: budget.max_agents,
        max_concurrency: budget.max_concurrency,
    }
}

fn campaign_budget_stop_reason(
    budget: &CampaignBudget,
    results: &[ModelTrialResult],
    threshold: f64,
) -> Option<String> {
    campaign_budget_violations(budget, results, threshold, None)
        .into_iter()
        .next()
        .map(|label| {
            format!(
                "shard reached {:.0}% of allocated campaign {label} budget",
                threshold * 100.0
            )
        })
}

fn campaign_budget_violations(
    budget: &CampaignBudget,
    results: &[ModelTrialResult],
    threshold: f64,
    wall_seconds_override: Option<u64>,
) -> Vec<String> {
    let wall_seconds = wall_seconds_override.unwrap_or_else(|| {
        results
            .iter()
            .map(|result| result.timings.wall_ms)
            .fold(0u64, u64::saturating_add)
            / 1_000
    });
    let model_calls = results
        .iter()
        .map(|result| result.orchestration.model_calls)
        .fold(0u64, u64::saturating_add);
    let input_tokens = results
        .iter()
        .filter_map(|result| result.tokens.input)
        .fold(0u64, u64::saturating_add);
    let output_tokens = results
        .iter()
        .filter_map(|result| result.tokens.output)
        .fold(0u64, u64::saturating_add);
    let tool_calls = results
        .iter()
        .map(|result| result.tools.attempted)
        .fold(0u64, u64::saturating_add);
    let cost = results
        .iter()
        .filter_map(|result| result.cost.total_usd)
        .sum::<f64>();
    let reached_u64 = |observed: u64, limit: Option<u64>| {
        limit.is_some_and(|limit| observed as f64 >= limit as f64 * threshold)
    };
    let reached_f64 = |observed: f64, limit: Option<f64>| {
        limit.is_some_and(|limit| observed >= limit * threshold)
    };
    let mut violations = Vec::new();
    if reached_u64(wall_seconds, budget.max_wall_seconds) {
        violations.push("wall_time".to_string());
    }
    if reached_u64(model_calls, budget.max_model_calls) {
        violations.push("model_calls".to_string());
    }
    if reached_u64(input_tokens, budget.max_input_tokens) {
        violations.push("input_tokens".to_string());
    }
    if reached_u64(output_tokens, budget.max_output_tokens) {
        violations.push("output_tokens".to_string());
    }
    if reached_u64(tool_calls, budget.max_tool_calls) {
        violations.push("tool_calls".to_string());
    }
    if reached_f64(cost, budget.max_cost_usd) {
        violations.push("cost".to_string());
    }
    if budget.max_input_tokens.is_some()
        && results
            .iter()
            .any(|result| result.orchestration.model_calls > 0 && result.tokens.input.is_none())
    {
        violations.push("input_tokens_unknown".to_string());
    }
    if budget.max_output_tokens.is_some()
        && results
            .iter()
            .any(|result| result.orchestration.model_calls > 0 && result.tokens.output.is_none())
    {
        violations.push("output_tokens_unknown".to_string());
    }
    if budget.max_cost_usd.is_some()
        && results
            .iter()
            .any(|result| result.orchestration.model_calls > 0 && result.cost.total_usd.is_none())
    {
        violations.push("cost_unknown".to_string());
    }
    violations
}

fn run_trial_subprocess(
    root: &Path,
    plan_path: &Path,
    trial: &PlannedModelTrial,
    planned_case: &PlannedModelCase,
    attempt: u8,
) -> Result<ModelTrialResult> {
    let temp = tempfile::tempdir().context("creating model trial subprocess directory")?;
    let result_path = temp.path().join("trial-result.json");
    let trial_home = temp.path().join("home");
    let trial_cache = trial_home.join(".cache");
    let trial_config = trial_home.join(".config");
    let trial_state = trial_home.join(".local/state");
    let trial_tmp = trial_home.join("tmp");
    for directory in [
        &trial_home,
        &trial_cache,
        &trial_config,
        &trial_state,
        &trial_tmp,
    ] {
        fs::create_dir_all(directory)?;
    }
    let executable = std::env::current_exe().context("resolving model eval executable")?;
    let mut command = Command::new(executable);
    command
        .arg("--root")
        .arg(root)
        .arg("model")
        .arg("_run-trial")
        .arg("--plan")
        .arg(plan_path)
        .arg("--trial")
        .arg(&trial.id)
        .arg("--output")
        .arg(&result_path)
        .env("HA_MODEL_EVAL_SUBPROCESS", "1")
        .env("HA_MODEL_EVAL_ATTEMPT", attempt.to_string())
        .env("HA_MODEL_EVAL_TRIAL_HOME", &trial_home)
        .env("HOME", &trial_home)
        .env("USERPROFILE", &trial_home)
        .env("XDG_CACHE_HOME", &trial_cache)
        .env("XDG_CONFIG_HOME", &trial_config)
        .env("XDG_STATE_HOME", &trial_state)
        .env("TMPDIR", &trial_tmp)
        .env("TMP", &trial_tmp)
        .env("TEMP", &trial_tmp)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    // The runner talks to a separately provisioned Hope process. Provider and
    // personal-service credentials must not leak into this harness subprocess.
    for (key, _) in std::env::vars() {
        let upper = key.to_ascii_uppercase();
        if key != "HA_MODEL_EVAL_SERVER_TOKEN"
            && key != "HA_MODEL_EVAL_SUPERVISOR_TOKEN"
            && (upper.ends_with("_API_KEY")
                || upper.ends_with("_TOKEN")
                || upper.contains("OPENAI")
                || upper.contains("ANTHROPIC")
                || upper.contains("PROVIDER_SECRET")
                || upper.contains("COOKIE"))
        {
            command.env_remove(key);
        }
    }
    let started_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let started = Instant::now();
    let mut child = command.spawn().context("spawning isolated model trial")?;
    let deadline = Duration::from_secs(planned_case.timeout_seconds);
    let process_status = loop {
        if let Some(status) = child.try_wait().context("polling model trial")? {
            break Some(status);
        }
        if started.elapsed() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        thread::sleep(Duration::from_millis(100));
    };
    if process_status.is_some_and(|status| status.success()) && result_path.is_file() {
        let result: ModelTrialResult = read_json(&result_path)?;
        validate_trial_attempt_result(trial, &result)?;
        return Ok(result);
    }
    let completed_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let (outcome, failure_class, error) = if process_status.is_none() {
        (
            ModelCampaignOutcome::BudgetExhausted,
            "trial_wall_timeout",
            format!(
                "model trial timed out after {} seconds",
                planned_case.timeout_seconds
            ),
        )
    } else {
        (
            ModelCampaignOutcome::InfraError,
            "trial_subprocess_failed",
            format!("model trial subprocess exited with {process_status:?}"),
        )
    };
    Ok(failed_trial_result(
        trial,
        attempt,
        &started_at,
        &completed_at,
        outcome,
        failure_class,
        &error,
        started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
    ))
}

async fn command_run_trial(
    root: &Path,
    plan_path: &Path,
    trial_id: &str,
    output: &Path,
) -> Result<()> {
    if std::env::var("HA_MODEL_EVAL_SUBPROCESS").as_deref() != Ok("1") {
        bail!("model trial worker may only run inside the isolated runner subprocess");
    }
    let plan: ModelCampaignPlan = read_json(plan_path)?;
    validate_execution_plan(root, &plan)?;
    let trial = plan
        .trials
        .iter()
        .find(|trial| trial.id == trial_id)
        .ok_or_else(|| anyhow!("model trial {trial_id} is not in plan"))?;
    let suite = plan
        .suites
        .iter()
        .find(|suite| suite.id == trial.suite_id)
        .ok_or_else(|| anyhow!("model trial suite is not in plan"))?;
    let planned_case = suite
        .cases
        .iter()
        .find(|case| case.id == trial.case_id)
        .ok_or_else(|| anyhow!("model trial case is not in plan"))?;
    let model = plan
        .models
        .iter()
        .find(|model| model.role == trial.model_role)
        .ok_or_else(|| anyhow!("model trial role is not in plan"))?;
    let (scenario_path, scenario) = load_scenario(root, &planned_case.scenario_path)?;
    let attempt = std::env::var("HA_MODEL_EVAL_ATTEMPT")
        .ok()
        .and_then(|value| value.parse::<u8>().ok())
        .filter(|value| (1..=2).contains(value))
        .unwrap_or(1);
    let result = crate::model_adapter::run_registered_trial(
        root,
        suite.adapter,
        &scenario_path,
        &scenario,
        planned_case,
        trial,
        model,
        attempt,
    )
    .await;
    validate_trial_attempt_result(trial, &result)?;
    validate_output_schema(root, "trial", &result)?;
    write_json(output, &result)
}

fn validate_trial_result(planned: &PlannedModelTrial, result: &ModelTrialResult) -> Result<()> {
    validate_trial_result_inner(planned, result, false)
}

/// Trial workers do not receive a previous attempt's payload. The parent
/// process validates an isolated attempt first, then appends the first-attempt
/// record and revalidates the complete retry chain before it enters a shard.
fn validate_trial_attempt_result(
    planned: &PlannedModelTrial,
    result: &ModelTrialResult,
) -> Result<()> {
    validate_trial_result_inner(planned, result, true)
}

fn validate_trial_result_inner(
    planned: &PlannedModelTrial,
    result: &ModelTrialResult,
    allow_unmerged_retry: bool,
) -> Result<()> {
    if result.schema_version != TRIAL_SCHEMA_VERSION
        || result.trial_id != planned.id
        || result.campaign_id != planned.campaign_id
        || result.plan_digest != planned.plan_digest
        || result.commit_sha != planned.commit_sha
        || result.app_version != planned.app_version
        || result.suite_id != planned.suite_id
        || result.suite_version != planned.suite_version
        || result.suite_digest != planned.suite_digest
        || result.case_id != planned.case_id
        || result.case_digest != planned.case_digest
        || result.trial_index != planned.trial_index
        || result.seed != planned.seed
        || result.model_role != planned.model_role
        || result.arm != planned.arm
        || result.fault_profile != planned.fault_profile
        || result.orchestration_profile != planned.orchestration_profile
        || result.model_digest != planned.model_digest
        || result.scenario_digest != planned.scenario_digest
        || result.environment_digest != planned.environment_digest
        || result.verifier_digest != planned.verifier_digest
        || result.prompt_digest != planned.prompt_digest
        || result.tool_schema_digest != planned.tool_schema_digest
    {
        bail!(
            "model trial {} metadata/digests do not match plan",
            planned.id
        );
    }
    if !(1..=2).contains(&result.attempt) {
        bail!("model trial {} has invalid attempt", result.trial_id);
    }
    if let Some(digest) = &result.runtime_config_digest {
        validate_sha256(digest, "model trial runtime config digest")?;
    }
    match (result.attempt, result.prior_attempts.as_slice()) {
        (1, []) => {}
        (2, []) if allow_unmerged_retry => {}
        (2, [prior])
            if prior.attempt == 1
                && matches!(
                    prior.outcome,
                    ModelCampaignOutcome::InfraError | ModelCampaignOutcome::SimulatorError
                ) =>
        {
            validate_attempt_record(&result.trial_id, prior)?;
        }
        _ => bail!(
            "model trial {} retry history is inconsistent",
            result.trial_id
        ),
    }
    let started = parse_timestamp(&result.started_at, "model trial startedAt")?;
    let completed = parse_timestamp(&result.completed_at, "model trial completedAt")?;
    let wall = wall_duration_ms(started, completed)?;
    // Monotonic measurement and wall-clock timestamps may differ slightly.
    if result.timings.wall_ms.abs_diff(wall) > 5_000 {
        bail!(
            "model trial {} wall timing is inconsistent",
            result.trial_id
        );
    }
    if result.tools.succeeded + result.tools.failed + result.tools.cancelled
        != result.tools.attempted
    {
        bail!(
            "model trial {} tool totals are inconsistent",
            result.trial_id
        );
    }
    if result.trace.orphan_span_count > 0 || !result.trace.closed {
        bail!("model trial {} trace is open or orphaned", result.trial_id);
    }
    let blocking_failed = result
        .milestones
        .iter()
        .chain(&result.invariants)
        .chain(&result.judge_checks)
        .any(|check| check.blocking && !check.passed);
    if result.outcome == ModelCampaignOutcome::Passed && blocking_failed {
        bail!(
            "model trial {} passed with a blocking failure",
            result.trial_id
        );
    }
    validate_trace_events(&result.trial_id, &result.trace, &result.trace_events)?;
    let mut artifact_paths = BTreeSet::new();
    for artifact in &result.artifacts {
        validate_relative_artifact(artifact, "model trial artifact")?;
        if !artifact_paths.insert(artifact.path.as_str()) {
            bail!("model trial contains a duplicate artifact path");
        }
    }
    Ok(())
}

fn validate_attempt_record(
    trial_id: &str,
    attempt: &ha_eval_spec::model::ModelTrialAttemptRecord,
) -> Result<()> {
    if let Some(digest) = &attempt.runtime_config_digest {
        validate_sha256(digest, "prior model attempt runtime config digest")?;
    }
    let started = parse_timestamp(&attempt.started_at, "model retry startedAt")?;
    let completed = parse_timestamp(&attempt.completed_at, "model retry completedAt")?;
    let wall = wall_duration_ms(started, completed)?;
    if attempt.timings.wall_ms.abs_diff(wall) > 5_000
        || attempt.tools.succeeded + attempt.tools.failed + attempt.tools.cancelled
            != attempt.tools.attempted
        || attempt.trace.orphan_span_count > 0
        || !attempt.trace.closed
    {
        bail!("model trial {trial_id} has an invalid prior attempt record");
    }
    validate_trace_events(trial_id, &attempt.trace, &attempt.trace_events)?;
    let mut artifacts = BTreeSet::new();
    for artifact in &attempt.artifacts {
        validate_relative_artifact(artifact, "prior model attempt artifact")?;
        if !artifacts.insert(artifact.path.as_str()) {
            bail!("model trial {trial_id} has duplicate prior-attempt artifacts");
        }
    }
    Ok(())
}

fn validate_trace_events(
    trial_id: &str,
    trace: &TraceSummary,
    events: &[ha_eval_spec::model::ModelCampaignEvent],
) -> Result<()> {
    validate_model_trace(trace, events, &format!("model trial {trial_id}"))
}

fn validate_relative_artifact(artifact: &ArtifactDigest, label: &str) -> Result<()> {
    let path = Path::new(&artifact.path);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        bail!("{label} path is unsafe");
    }
    validate_sha256(&artifact.sha256, &format!("{label} digest"))
}

fn resolve_shard_trace_path(shard_path: &Path, artifact: &ArtifactDigest) -> Result<PathBuf> {
    validate_relative_artifact(artifact, "model shard trace artifact")?;
    let parent = shard_path
        .parent()
        .ok_or_else(|| anyhow!("model shard path has no parent"))?
        .canonicalize()
        .context("canonicalizing model shard directory")?;
    let candidate = parent.join(&artifact.path);
    let metadata = fs::symlink_metadata(&candidate)
        .with_context(|| format!("reading model trace artifact {}", candidate.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!("model shard trace artifact must be a regular non-symlink file");
    }
    let canonical = candidate
        .canonicalize()
        .context("canonicalizing model shard trace artifact")?;
    if !canonical.starts_with(&parent) {
        bail!("model shard trace artifact escapes its shard directory");
    }
    Ok(canonical)
}

fn enforce_runtime_network_contract(policy: NetworkPolicy) -> Result<()> {
    let declared = std::env::var("HA_MODEL_EVAL_NETWORK_POLICY").ok();
    let expected = serde_json::to_value(policy)?
        .as_str()
        .ok_or_else(|| anyhow!("network policy did not serialize as a string"))?
        .to_string();
    if is_dedicated_source() && declared.as_deref() != Some(expected.as_str()) {
        bail!(
            "dedicated runner must attest HA_MODEL_EVAL_NETWORK_POLICY={expected}; actual={}",
            declared.as_deref().unwrap_or("unset")
        );
    }
    if is_dedicated_source()
        && std::env::var("HA_MODEL_EVAL_NETWORK_ENFORCED").as_deref() != Ok("1")
    {
        bail!(
            "dedicated runner must attest that its external egress policy is enforced with HA_MODEL_EVAL_NETWORK_ENFORCED=1"
        );
    }
    if matches!(policy, NetworkPolicy::LiveWeb)
        && std::env::var("HA_MODEL_EVAL_ALLOW_LIVE_WEB").as_deref() != Ok("1")
    {
        bail!("live_web scenarios require an explicit HA_MODEL_EVAL_ALLOW_LIVE_WEB=1 opt-in");
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn failed_trial_result(
    trial: &PlannedModelTrial,
    attempt: u8,
    started_at: &str,
    completed_at: &str,
    outcome: ModelCampaignOutcome,
    failure_class: &str,
    error: &str,
    wall_ms: u64,
) -> ModelTrialResult {
    ModelTrialResult {
        schema_version: TRIAL_SCHEMA_VERSION.to_string(),
        trial_id: trial.id.clone(),
        campaign_id: trial.campaign_id.clone(),
        plan_digest: trial.plan_digest.clone(),
        commit_sha: trial.commit_sha.clone(),
        app_version: trial.app_version.clone(),
        suite_id: trial.suite_id.clone(),
        suite_version: trial.suite_version.clone(),
        suite_digest: trial.suite_digest.clone(),
        case_id: trial.case_id.clone(),
        case_digest: trial.case_digest.clone(),
        trial_index: trial.trial_index,
        seed: trial.seed,
        model_role: trial.model_role.clone(),
        arm: trial.arm.clone(),
        fault_profile: trial.fault_profile,
        orchestration_profile: trial.orchestration_profile.clone(),
        model_digest: trial.model_digest.clone(),
        scenario_digest: trial.scenario_digest.clone(),
        environment_digest: trial.environment_digest.clone(),
        verifier_digest: trial.verifier_digest.clone(),
        prompt_digest: trial.prompt_digest.clone(),
        tool_schema_digest: trial.tool_schema_digest.clone(),
        runtime_config_digest: current_runtime_config_digest(),
        attempt,
        prior_attempts: Vec::new(),
        started_at: started_at.to_string(),
        completed_at: completed_at.to_string(),
        outcome,
        failure_class: Some(failure_class.to_string()),
        attribution: AttributionCompleteness::Missing,
        timings: TimingMetrics {
            wall_ms,
            critical_path_ms: wall_ms,
            ..Default::default()
        },
        tokens: TokenMetrics::default(),
        cost: CostMetrics::default(),
        tools: ToolMetrics::default(),
        orchestration: OrchestrationMetrics::default(),
        milestones: Vec::new(),
        invariants: Vec::new(),
        judge_checks: Vec::new(),
        trace: TraceSummary {
            trace_id: format!("trace_{}", trial.id),
            root_span_id: format!("span_{}", &trial.id[trial.id.len().saturating_sub(16)..]),
            span_count: 1,
            orphan_span_count: 0,
            closed: true,
        },
        trace_events: Vec::new(),
        artifacts: Vec::new(),
        warnings: vec!["trial failed before complete product attribution".to_string()],
        error: Some(sanitize_error(error)),
    }
}

fn command_aggregate(
    root: &Path,
    plan_path: &Path,
    inputs: &[PathBuf],
    output: &Path,
    summary: &Path,
    waiver_path: Option<&Path>,
) -> Result<()> {
    let plan: ModelCampaignPlan = read_json(plan_path)?;
    let app_plan = validate_execution_plan(root, &plan)?;
    let policy = app_plan
        .is_none()
        .then(|| load_policy(root, plan.tier))
        .transpose()?;
    let shard_files = collect_json_files(inputs)?;
    let bundle_root = output.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(bundle_root.join("shards"))?;
    fs::create_dir_all(bundle_root.join("traces"))?;
    let mut artifacts = Vec::new();
    let mut found = BTreeMap::<String, ModelTrialResult>::new();
    let mut shards = BTreeSet::<(String, u16)>::new();
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
        let shard: ModelShardResult = serde_json::from_value(value)
            .with_context(|| format!("parsing model shard result {}", path.display()))?;
        validate_model_shard(&plan, &shard)?;
        let trace_path = resolve_shard_trace_path(&path, &shard.trace_artifact)?;
        let actual_trace = fs::read_to_string(&trace_path)
            .with_context(|| format!("reading model trace artifact {}", trace_path.display()))?;
        if digest_file(&trace_path)? != shard.trace_artifact.sha256
            || actual_trace != render_trace_jsonl(&shard.trials)?
        {
            bail!("model shard trace artifact does not match its signed trial events");
        }
        if !shards.insert((shard.suite_id.clone(), shard.shard_index)) {
            bail!(
                "duplicate model shard result for {}/{}",
                shard.suite_id,
                shard.shard_index
            );
        }
        let shard_started_at = parse_timestamp(&shard.started_at, "model shard startedAt")?;
        let shard_completed_at = parse_timestamp(&shard.completed_at, "model shard completedAt")?;
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
        let shard_label = format!("shards/{}-shard-{}.json", shard.suite_id, shard.shard_index);
        let trace_label = format!(
            "traces/{}-shard-{}.jsonl",
            shard.suite_id, shard.shard_index
        );
        let bundled_shard = bundle_root.join(&shard_label);
        let bundled_trace = bundle_root.join(&trace_label);
        fs::copy(&path, &bundled_shard)
            .with_context(|| format!("bundling verified model shard {}", path.display()))?;
        fs::copy(&trace_path, &bundled_trace)
            .with_context(|| format!("bundling verified model trace {}", trace_path.display()))?;
        artifacts.push(ArtifactDigest {
            path: shard_label,
            sha256: digest_file(&bundled_shard)?,
        });
        artifacts.push(ArtifactDigest {
            path: trace_label,
            sha256: digest_file(&bundled_trace)?,
        });
        for trial in shard.trials {
            if found.insert(trial.trial_id.clone(), trial).is_some() {
                bail!("duplicate result for model trial");
            }
        }
    }
    let fallback_time = latest_completed_at.unwrap_or_else(Utc::now);
    let fallback_timestamp = fallback_time.to_rfc3339_opts(SecondsFormat::Millis, true);
    let mut trial_results = Vec::with_capacity(plan.trials.len());
    for planned in &plan.trials {
        let result = found.remove(&planned.id).unwrap_or_else(|| {
            failed_trial_result(
                planned,
                1,
                &fallback_timestamp,
                &fallback_timestamp,
                ModelCampaignOutcome::InfraError,
                "missing_shard_result",
                "planned model trial has no shard result",
                0,
            )
        });
        validate_trial_result(planned, &result)?;
        trial_results.push(result);
    }
    if !found.is_empty() {
        bail!("model shard results contain trials not present in plan");
    }
    trial_results.sort_by(|left, right| left.trial_id.cmp(&right.trial_id));
    artifacts.sort_by(|left, right| left.path.cmp(&right.path));
    let completed_at = latest_completed_at.unwrap_or(fallback_time);
    let started_at = earliest_started_at.unwrap_or(completed_at);
    let duration_ms = wall_duration_ms(started_at, completed_at)?;
    let source = current_source();
    if policy
        .as_ref()
        .is_some_and(|policy| !policy.allowed_sources.contains(&source))
        && !matches!(
            source,
            ModelCampaignSource::LocalApp | ModelCampaignSource::LocalCli
        )
    {
        bail!("current model evidence source is not allowed by policy");
    }
    if app_plan.is_some() && waiver_path.is_some() {
        bail!("local App evidence does not accept release waivers");
    }
    let waiver = waiver_path
        .map(read_json::<ModelCampaignWaiver>)
        .transpose()?;
    if let Some(waiver) = &waiver {
        validate_model_waiver(waiver, &plan.reference, None)?;
    }
    let guardrail_violations = trial_results
        .iter()
        .flat_map(|result| {
            result
                .invariants
                .iter()
                .filter(|check| check.blocking && !check.passed)
                .map(move |check| format!("{}/{}:{}", result.suite_id, result.case_id, check.id))
        })
        .collect::<Vec<_>>();
    let campaign_budget_violations = campaign_budget_violations(
        &plan.campaign_budget,
        &trial_results,
        1.0,
        Some(duration_ms / 1_000),
    );
    let mut warnings = Vec::new();
    if trial_results
        .iter()
        .any(|result| result.attribution != AttributionCompleteness::Complete)
    {
        warnings.push("one or more trials have incomplete attribution".to_string());
    }
    if trial_results
        .iter()
        .any(|result| result.cost.total_usd.is_none())
    {
        warnings.push("one or more trials have unknown cost".to_string());
    }
    if policy
        .as_ref()
        .is_none_or(|policy| !policy.allowed_sources.contains(&source))
    {
        warnings.push(
            "local evidence source is not policy-eligible and cannot be promoted to release"
                .to_string(),
        );
    }
    if !campaign_budget_violations.is_empty() {
        warnings.push(format!(
            "campaign budget exceeded: {}",
            campaign_budget_violations.join(", ")
        ));
    }
    let aggregate_status = if campaign_budget_violations.is_empty() {
        aggregate_model_status(&trial_results)
    } else {
        EvalStatus::Failed
    };
    let evidence = ModelCampaignEvidence {
        schema_version: EVIDENCE_SCHEMA_VERSION.to_string(),
        evidence_kind: "model_campaign".to_string(),
        campaign_id: stable_campaign_id(&plan),
        commit_sha: plan.reference.clone(),
        dirty: app_plan
            .as_ref()
            .map_or_else(|| git_dirty(root), |plan| plan.dirty),
        source,
        app_version: plan.app_version.clone(),
        tier: plan.tier,
        policy_id: plan.policy_id.clone(),
        policy_version: plan.policy_version.clone(),
        policy_mode: policy
            .as_ref()
            .map_or(PolicyMode::Advisory, |policy| policy.mode),
        policy_digest: plan.policy_digest.clone(),
        runner_digest: plan.runner_digest.clone(),
        runner_os: std::env::consts::OS.to_string(),
        runner_arch: std::env::consts::ARCH.to_string(),
        execution_profile: app_plan.as_ref().map(|plan| plan.execution_profile),
        app_plan_digest: app_plan.as_ref().map(|plan| plan.plan_digest.clone()),
        runtime_environment: app_plan
            .as_ref()
            .map(|plan| plan.runtime_environment.clone()),
        aggregate_status,
        started_at: started_at.to_rfc3339_opts(SecondsFormat::Millis, true),
        completed_at: completed_at.to_rfc3339_opts(SecondsFormat::Millis, true),
        duration_ms,
        counts: aggregate_counts(plan.trials.len(), &trial_results),
        metrics: aggregate_metrics(&plan.trials, &trial_results),
        models: plan.models.clone(),
        campaign_budget: plan.campaign_budget.clone(),
        suites: plan.suites.clone(),
        planned_trials: plan.trials.clone(),
        trial_results,
        campaign_budget_violations,
        guardrail_violations,
        regressions: Vec::new(),
        warnings,
        artifacts,
        waiver,
    };
    validate_evidence_shape(&evidence)?;
    validate_output_schema(root, "evidence", &evidence)?;
    scan_evidence_for_secrets(&serde_json::to_value(&evidence)?)?;
    write_json(output, &evidence)?;
    if let Some(parent) = summary.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(summary, evidence_markdown(&evidence))
        .with_context(|| format!("writing model summary {}", summary.display()))?;
    println!(
        "aggregated {} model trials: {:?} ({:.2}s)",
        evidence.trial_results.len(),
        evidence.aggregate_status,
        evidence.duration_ms as f64 / 1_000.0
    );
    Ok(())
}

fn validate_model_shard(plan: &ModelCampaignPlan, shard: &ModelShardResult) -> Result<()> {
    if shard.schema_version != SHARD_SCHEMA_VERSION
        || shard.reference != plan.reference
        || shard.runner_digest != plan.runner_digest
    {
        bail!("model shard schema/ref/runner does not match plan");
    }
    let suite = plan
        .suites
        .iter()
        .find(|suite| suite.id == shard.suite_id)
        .ok_or_else(|| anyhow!("model shard suite is not in plan"))?;
    if shard.suite_digest != suite.digest
        || shard.shard_total != suite.shards
        || !(1..=shard.shard_total).contains(&shard.shard_index)
    {
        bail!("model shard metadata does not match suite {}", suite.id);
    }
    let started = parse_timestamp(&shard.started_at, "model shard startedAt")?;
    let completed = parse_timestamp(&shard.completed_at, "model shard completedAt")?;
    if shard.duration_ms != wall_duration_ms(started, completed)? {
        bail!("model shard duration does not match timestamps");
    }
    validate_relative_artifact(&shard.trace_artifact, "model shard trace artifact")?;
    let mut ids = BTreeSet::new();
    for result in &shard.trials {
        if !ids.insert(result.trial_id.as_str()) {
            bail!("model shard contains a duplicate trial");
        }
        let planned = plan
            .trials
            .iter()
            .find(|trial| trial.id == result.trial_id)
            .ok_or_else(|| anyhow!("model shard contains an unplanned trial"))?;
        if planned.suite_id != suite.id
            || stable_shard(&planned.id, suite.shards) + 1 != shard.shard_index
        {
            bail!("model trial is in the wrong shard");
        }
        validate_trial_result(planned, result)?;
    }
    Ok(())
}

fn command_verify_evidence(
    root: &Path,
    evidence_path: &Path,
    reference: &str,
    tier: ModelCampaignTier,
    tag: Option<&str>,
    allow_local: bool,
) -> Result<()> {
    validate_git_reference(reference)?;
    let raw: Value = read_json(evidence_path)?;
    let schema: Value =
        read_json(&live_root(root).join("schema/eval-model-campaign-v1.schema.json"))?;
    validate_json_schema(&raw, &schema).context("validating model campaign evidence schema")?;
    scan_evidence_for_secrets(&raw)?;
    let evidence: ModelCampaignEvidence = serde_json::from_value(raw)?;
    validate_evidence_shape(&evidence)?;
    verify_evidence_artifact_files(evidence_path, &evidence.artifacts)?;
    validate_release_source_identity(
        &evidence.commit_sha,
        evidence.tier,
        &evidence.runner_digest,
        evidence.dirty,
        evidence.source,
        reference,
        tier,
        allow_local,
    )?;
    let policy = load_policy(root, tier)?;
    if !policy.allowed_sources.contains(&evidence.source) && !allow_local {
        bail!("model evidence source is forbidden by policy");
    }
    if evidence.policy_id != policy.id
        || evidence.policy_version != policy.version
        || evidence.policy_mode != policy.mode
        || evidence.policy_digest != digest_serializable(&policy)?
    {
        bail!("model evidence policy does not match repository policy");
    }
    let expected_plan = build_plan(root, tier, reference)?;
    if evidence.app_version != expected_plan.app_version
        || evidence.models != expected_plan.models
        || evidence.campaign_budget != expected_plan.campaign_budget
        || evidence.suites != expected_plan.suites
        || evidence.planned_trials != expected_plan.trials
    {
        bail!("model evidence models/assets/trials do not match current plan");
    }
    if evidence.campaign_id != stable_campaign_id(&expected_plan) {
        bail!("model evidence campaign id is not derived from the immutable plan");
    }
    if let Some(tag) = tag {
        let tag_version = tag.strip_prefix('v').unwrap_or(tag);
        if tag_version != evidence.app_version {
            bail!("model evidence app version does not match release tag {tag}");
        }
    }
    let expected_by_id = expected_plan
        .trials
        .iter()
        .map(|trial| (trial.id.as_str(), trial))
        .collect::<BTreeMap<_, _>>();
    for result in &evidence.trial_results {
        let planned = expected_by_id
            .get(result.trial_id.as_str())
            .ok_or_else(|| anyhow!("model evidence contains an unplanned trial"))?;
        validate_trial_result(planned, result)?;
    }
    let started = parse_timestamp(&evidence.started_at, "model evidence startedAt")?;
    let completed = parse_timestamp(&evidence.completed_at, "model evidence completedAt")?;
    if evidence.duration_ms != wall_duration_ms(started, completed)? {
        bail!("model evidence duration does not match timestamps");
    }
    let expected_artifacts = expected_plan
        .suites
        .iter()
        .map(|suite| usize::from(suite.shards) * 2)
        .sum::<usize>();
    if evidence.artifacts.len() != expected_artifacts {
        bail!("model evidence must include one artifact digest per planned shard");
    }
    let mut artifact_paths = BTreeSet::new();
    for artifact in &evidence.artifacts {
        if !artifact_paths.insert(artifact.path.as_str()) {
            bail!("model evidence contains duplicate artifact paths");
        }
        validate_sha256(&artifact.sha256, "model evidence artifact digest")?;
    }
    if policy.mode == PolicyMode::Enforce {
        enforce_model_campaign_readiness(&evidence, &policy, tier)?;
    }
    let failed_suites = failed_model_suites(&evidence);
    if let Some(waiver) = &evidence.waiver {
        if tier != ModelCampaignTier::Release {
            bail!("model waivers are only valid for release evidence");
        }
        let tag = tag.ok_or_else(|| anyhow!("waived model evidence requires a release tag"))?;
        validate_model_waiver(waiver, reference, Some(tag))?;
        let waived = waiver
            .suites
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let planned = expected_plan
            .suites
            .iter()
            .map(|suite| suite.id.as_str())
            .collect::<BTreeSet<_>>();
        if waived.iter().any(|suite| !planned.contains(*suite))
            || failed_suites
                .iter()
                .any(|suite| !waived.contains(suite.as_str()))
        {
            bail!("model waiver does not cover exactly planned failed suites");
        }
    }
    if policy.mode == PolicyMode::Enforce {
        if !evidence.guardrail_violations.is_empty() {
            bail!("blocking model guardrail violations cannot be waived");
        }
        if !evidence.campaign_budget_violations.is_empty() {
            bail!("model campaign budget violations cannot be waived");
        }
        if !failed_suites.is_empty() && evidence.waiver.is_none() {
            bail!(
                "enforced model policy failed suites: {}",
                failed_suites.join(", ")
            );
        }
    }
    println!(
        "verified {:?} model evidence for {} (source {:?}, policy {:?})",
        evidence.aggregate_status, reference, evidence.source, policy.mode
    );
    Ok(())
}

fn enforce_model_campaign_readiness(
    evidence: &ModelCampaignEvidence,
    policy: &ModelCampaignPolicy,
    tier: ModelCampaignTier,
) -> Result<()> {
    let infra_rate = evidence.metrics.infra_error_rate.unwrap_or(1.0);
    if infra_rate > policy.max_infra_error_rate {
        bail!(
            "model evidence infra error rate {:.4} exceeds policy {:.4}",
            infra_rate,
            policy.max_infra_error_rate
        );
    }
    if evidence.counts.benchmark_defect > 0
        || evidence.counts.simulator_error > 0
        || evidence.counts.cancelled > 0
    {
        bail!(
            "model evidence contains invalid trials (benchmark_defect={}, simulator_error={}, cancelled={})",
            evidence.counts.benchmark_defect,
            evidence.counts.simulator_error,
            evidence.counts.cancelled
        );
    }
    if tier == ModelCampaignTier::Release
        && evidence
            .trial_results
            .iter()
            .any(|result| result.attribution != AttributionCompleteness::Complete)
    {
        bail!("release model evidence requires complete attribution for every trial");
    }
    if tier == ModelCampaignTier::Release
        && evidence.trial_results.iter().any(|result| {
            result.outcome.is_valid_trial()
                && (result.cost.total_usd.is_none()
                    || result
                        .cost
                        .price_snapshot_digest
                        .as_deref()
                        .is_none_or(str::is_empty))
        })
    {
        bail!(
            "release model evidence requires priced valid trials and an immutable price snapshot digest"
        );
    }
    if tier == ModelCampaignTier::Release {
        let runtime_config_digests = evidence
            .trial_results
            .iter()
            .filter(|result| result.outcome.is_valid_trial())
            .filter_map(|result| result.runtime_config_digest.as_deref())
            .collect::<BTreeSet<_>>();
        if evidence
            .trial_results
            .iter()
            .any(|result| result.outcome.is_valid_trial() && result.runtime_config_digest.is_none())
            || runtime_config_digests.len() != 1
        {
            bail!(
                "release model evidence requires one immutable runtime config digest across all valid trials"
            );
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_release_source_identity(
    commit_sha: &str,
    evidence_tier: ModelCampaignTier,
    runner_digest: &str,
    dirty: bool,
    source: ModelCampaignSource,
    reference: &str,
    requested_tier: ModelCampaignTier,
    allow_local: bool,
) -> Result<()> {
    if commit_sha != reference
        || evidence_tier != requested_tier
        || runner_digest != model_runner_digest(reference)
    {
        bail!("model evidence ref/tier/runner does not match release request");
    }
    if dirty && !allow_local {
        bail!("release model evidence was produced from a dirty worktree");
    }
    if !allow_local && !source.is_release_eligible() {
        bail!("release model evidence must come from GitHub Actions or a dedicated runner");
    }
    Ok(())
}

fn failed_model_suites(evidence: &ModelCampaignEvidence) -> Vec<String> {
    evidence
        .suites
        .iter()
        .filter(|suite| {
            evidence.trial_results.iter().any(|result| {
                result.suite_id == suite.id && result.outcome != ModelCampaignOutcome::Passed
            })
        })
        .map(|suite| suite.id.clone())
        .collect()
}

fn validate_model_waiver(
    waiver: &ModelCampaignWaiver,
    reference: &str,
    tag: Option<&str>,
) -> Result<()> {
    if waiver.schema_version != WAIVER_SCHEMA_VERSION
        || waiver.commit_sha != reference
        || waiver.reason.trim().len() < 12
        || waiver.suites.is_empty()
        || waiver.approved_by.trim().is_empty()
        || waiver.workflow_run_id.trim().is_empty()
    {
        bail!("model waiver schema/ref/audit fields are invalid");
    }
    if tag.is_some_and(|tag| tag != waiver.tag) {
        bail!("model waiver tag does not match release tag");
    }
    parse_timestamp(&waiver.approved_at, "model waiver approvedAt")?;
    let mut suites = BTreeSet::new();
    if waiver.suites.iter().any(|suite| !suites.insert(suite)) {
        bail!("model waiver contains duplicate suites");
    }
    Ok(())
}

fn collect_json_files(inputs: &[PathBuf]) -> Result<Vec<PathBuf>> {
    fn visit(path: &Path, output: &mut Vec<PathBuf>) -> Result<()> {
        if path.is_dir() {
            for entry in fs::read_dir(path)
                .with_context(|| format!("reading model shard directory {}", path.display()))?
            {
                visit(&entry?.path(), output)?;
            }
        } else if path.extension().and_then(|value| value.to_str()) == Some("json") {
            output.push(path.to_path_buf());
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

fn verify_evidence_artifact_files(
    evidence_path: &Path,
    artifacts: &[ArtifactDigest],
) -> Result<()> {
    let root = evidence_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .canonicalize()
        .context("canonicalizing model evidence artifact directory")?;
    for artifact in artifacts {
        validate_relative_artifact(artifact, "model evidence artifact")?;
        let candidate = root.join(&artifact.path);
        let metadata = fs::symlink_metadata(&candidate)
            .with_context(|| format!("reading model evidence artifact {}", candidate.display()))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            bail!("model evidence artifact must be a regular non-symlink file");
        }
        let canonical = candidate
            .canonicalize()
            .context("canonicalizing model evidence artifact")?;
        if !canonical.starts_with(&root) || digest_file(&canonical)? != artifact.sha256 {
            bail!("model evidence artifact path or digest does not match evidence");
        }
    }
    Ok(())
}

fn current_source() -> ModelCampaignSource {
    if std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true") {
        ModelCampaignSource::GithubActions
    } else if std::env::var("HA_MODEL_EVAL_DEDICATED_RUNNER").as_deref() == Ok("1") {
        ModelCampaignSource::DedicatedRunner
    } else if std::env::var(APP_CONTROL_ENV).as_deref() == Ok("1")
        && std::env::var_os(APP_PLAN_ENV).is_some()
        && std::env::var("HA_MODEL_EVAL_SOURCE").as_deref() == Ok("local_app")
    {
        ModelCampaignSource::LocalApp
    } else {
        ModelCampaignSource::LocalCli
    }
}

fn is_dedicated_source() -> bool {
    matches!(
        current_source(),
        ModelCampaignSource::DedicatedRunner | ModelCampaignSource::GithubActions
    )
}

fn stable_campaign_id(plan: &ModelCampaignPlan) -> String {
    plan.campaign_id.clone()
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

fn evidence_markdown(evidence: &ModelCampaignEvidence) -> String {
    let mut output = format!(
        "# Model Campaign Evidence\n\n- Status: `{:?}`\n- Commit: `{}`\n- Tier: `{:?}`\n- Policy: `{}` `{}` (`{:?}`)\n- Source: `{:?}`\n- Dirty: `{}`\n- Trials: `{}` scheduled / `{}` valid / `{}` passed\n- Retries: `{}` retried / `{}` recovered\n- Hard success: `{}`\n- End-to-end yield: `{}`\n- Duration: `{:.2}s`\n\n| Suite | Passed | Failed | Infra/invalid |\n|---|---:|---:|---:|\n",
        evidence.aggregate_status,
        evidence.commit_sha,
        evidence.tier,
        evidence.policy_id,
        evidence.policy_version,
        evidence.policy_mode,
        evidence.source,
        evidence.dirty,
        evidence.counts.scheduled,
        evidence.counts.valid,
        evidence.counts.passed,
        evidence.counts.retried,
        evidence.counts.recovered_after_retry,
        format_ratio(evidence.metrics.hard_success_rate),
        format_ratio(evidence.metrics.end_to_end_yield),
        evidence.duration_ms as f64 / 1_000.0,
    );
    for suite in &evidence.suites {
        let mut passed = 0;
        let mut failed = 0;
        let mut infra = 0;
        for result in evidence
            .trial_results
            .iter()
            .filter(|result| result.suite_id == suite.id)
        {
            match result.outcome.aggregate_status() {
                Some(EvalStatus::Passed) => passed += 1,
                Some(EvalStatus::Failed) => failed += 1,
                _ => infra += 1,
            }
        }
        output.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            suite.id, passed, failed, infra
        ));
    }
    if !evidence.metrics.comparisons.is_empty() {
        output.push_str(
            "\n## Compute-matched orchestration comparisons\n\n| Case | Fault profile | Paired | Solo success | Team success | Uplift | Wall speedup p50 | Token amp p50 |\n|---|---|---:|---:|---:|---:|---:|---:|\n",
        );
        for comparison in &evidence.metrics.comparisons {
            output.push_str(&format!(
                "| {} | {:?} | {} | {} | {} | {} | {} | {} |\n",
                comparison.case_id,
                comparison.fault_profile,
                comparison.paired_valid_trials,
                format_ratio(comparison.baseline_success_rate),
                format_ratio(comparison.candidate_success_rate),
                comparison
                    .candidate_uplift_pp
                    .map(|value| format!("{value:.2} pp"))
                    .unwrap_or_else(|| "n/a".to_string()),
                format_optional_number(comparison.successful_wall_speedup.p50),
                format_optional_number(comparison.successful_token_amplification.p50),
            ));
        }
    }
    if !evidence.guardrail_violations.is_empty() {
        output.push_str("\n## Blocking guardrail violations\n\n");
        for violation in &evidence.guardrail_violations {
            output.push_str(&format!("- `{violation}`\n"));
        }
    }
    if !evidence.warnings.is_empty() {
        output.push_str("\n## Warnings\n\n");
        for warning in &evidence.warnings {
            output.push_str(&format!("- {}\n", sanitize_error(warning)));
        }
    }
    if let Some(waiver) = &evidence.waiver {
        output.push_str(&format!(
            "\n## Audited model waiver\n\nApproved by `{}` for `{}`: {}\n",
            waiver.approved_by,
            waiver.tag,
            sanitize_error(&waiver.reason)
        ));
    }
    output
}

fn format_ratio(value: Option<f64>) -> String {
    value
        .map(|value| format!("{:.2}%", value * 100.0))
        .unwrap_or_else(|| "n/a".to_string())
}

fn format_optional_number(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:.3}"))
        .unwrap_or_else(|| "n/a".to_string())
}

fn sanitize_error(value: &str) -> String {
    let mut output = value.replace(['\r', '\n'], " ");
    for marker in ["bearer ", "sk-", "api_key=", "token="] {
        loop {
            let lowercase = output.to_ascii_lowercase();
            let Some(start) = lowercase.find(marker) else {
                break;
            };
            let end = output[start..]
                .find(char::is_whitespace)
                .map(|offset| start + offset)
                .unwrap_or(output.len());
            output.replace_range(start..end, "[REDACTED]");
        }
    }
    output.truncate(2_000);
    output
}

fn scan_evidence_for_secrets(value: &Value) -> Result<()> {
    fn visit(value: &Value, location: &str) -> Result<()> {
        match value {
            Value::Object(object) => {
                for (key, child) in object {
                    let normalized = key.to_ascii_lowercase().replace(['_', '-'], "");
                    if matches!(
                        normalized.as_str(),
                        "apikey"
                            | "authorization"
                            | "cookie"
                            | "accesstoken"
                            | "refreshtoken"
                            | "privatekey"
                            | "clientsecret"
                    ) {
                        bail!(
                            "model evidence contains a forbidden secret field at {location}.{key}"
                        );
                    }
                    visit(child, &format!("{location}.{key}"))?;
                }
            }
            Value::Array(items) => {
                for (index, child) in items.iter().enumerate() {
                    visit(child, &format!("{location}[{index}]"))?;
                }
            }
            Value::String(string) => {
                let lower = string.to_ascii_lowercase();
                if lower.contains("-----begin private key-----")
                    || lower.contains("authorization: bearer ")
                    || lower.contains("x-api-key:")
                    || looks_like_provider_key(string)
                {
                    bail!("model evidence secret scan failed at {location}");
                }
                if string.contains("/Users/")
                    || string.contains("C:\\Users\\")
                    || string.contains("/home/")
                {
                    bail!("model evidence contains a personal absolute path at {location}");
                }
            }
            _ => {}
        }
        Ok(())
    }
    visit(value, "$")
}

fn looks_like_provider_key(value: &str) -> bool {
    let compact = value.trim();
    (compact.starts_with("sk-") && compact.len() >= 20)
        || (compact.starts_with("sk-ant-") && compact.len() >= 24)
        || (compact.starts_with("AIza") && compact.len() >= 30)
}

#[allow(clippy::too_many_arguments)]
fn validate_suite_policy_compatibility(
    policy: &ModelCampaignPolicy,
    suite: &ModelCampaignSuite,
    selected: &ModelPolicySuite,
    allowed_adapters: &BTreeSet<ModelCampaignAdapter>,
    allowed_runners: &BTreeSet<RunnerClass>,
    allowed_network: &BTreeSet<NetworkPolicy>,
    allowed_modes: &BTreeSet<ExecutionMode>,
) -> Result<()> {
    if !suite.tiers.contains(&policy.tier) {
        bail!("model suite {} does not support selected tier", suite.id);
    }
    if !allowed_adapters.contains(&suite.adapter)
        || !allowed_runners.contains(&suite.runner_class)
        || !allowed_network.contains(&suite.network_policy)
        || !allowed_modes.contains(&suite.execution_mode)
    {
        bail!("model suite {} exceeds policy allowlists", suite.id);
    }
    if suite.scorer.llm_judge && !policy.allow_llm_judge {
        bail!(
            "model suite {} requires an LLM Judge forbidden by policy",
            suite.id
        );
    }
    if selected.required
        && policy.mode == PolicyMode::Enforce
        && suite.execution_mode != ExecutionMode::NativeProvider
    {
        bail!("enforced required suites must use native_provider");
    }
    Ok(())
}

fn validate_scenario_policy_compatibility(
    policy: &ModelCampaignPolicy,
    suite: &ModelCampaignSuite,
    case: &ModelCampaignCaseSpec,
    scenario: &LiveAgentScenario,
    models: &BTreeMap<&str, &ModelProfile>,
) -> Result<()> {
    if scenario.environment.runner_class != suite.runner_class
        || scenario.network.policy != suite.network_policy
        || !scenario.cadence.contains(&policy.tier)
    {
        bail!(
            "scenario {} runner/network/cadence does not match suite",
            scenario.id
        );
    }
    if !scenario
        .verifiers
        .iter()
        .any(|verifier| verifier.handler == suite.scorer.hard_verifier && verifier.blocking)
    {
        bail!(
            "scenario {} does not contain the suite hard verifier",
            scenario.id
        );
    }
    let roles = if case.model_roles.is_empty() {
        models.keys().copied().collect::<Vec<_>>()
    } else {
        case.model_roles.iter().map(String::as_str).collect()
    };
    for role in roles {
        if !models.contains_key(role) {
            bail!(
                "scenario {} references unknown model role {role}",
                scenario.id
            );
        }
    }
    if let Some(role) = scenario.user_simulator.model_role.as_deref() {
        if !models.contains_key(role) {
            bail!(
                "scenario {} user simulator references unknown role {role}",
                scenario.id
            );
        }
    }
    if policy.tier == ModelCampaignTier::Release
        && scenario.user_simulator.kind == UserSimulatorKind::Llm
    {
        bail!("release v1 forbids LLM user simulators");
    }
    Ok(())
}

fn validate_git_reference(reference: &str) -> Result<()> {
    let reference = reference.trim();
    if !matches!(reference.len(), 40 | 64) || !reference.chars().all(|ch| ch.is_ascii_hexdigit()) {
        bail!("model campaign ref must be an exact 40- or 64-character commit SHA");
    }
    Ok(())
}

fn validate_sha256(value: &str, label: &str) -> Result<()> {
    if value.len() != 64 || !value.chars().all(|ch| ch.is_ascii_hexdigit()) {
        bail!("{label} is not a SHA-256 digest");
    }
    Ok(())
}

fn app_version(root: &Path) -> Result<String> {
    let package: Value = read_json(&root.join("package.json"))?;
    package
        .get("version")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("package.json does not contain version"))
}

#[allow(dead_code)]
fn assert_protocol_constants_are_distinct() {
    let versions = [
        SCENARIO_SCHEMA_VERSION,
        SUITE_SCHEMA_VERSION,
        POLICY_SCHEMA_VERSION,
        PLAN_SCHEMA_VERSION,
        TRIAL_SCHEMA_VERSION,
        EVIDENCE_SCHEMA_VERSION,
    ];
    debug_assert_eq!(
        versions.into_iter().collect::<BTreeSet<_>>().len(),
        versions.len()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn planned_trial() -> PlannedModelTrial {
        PlannedModelTrial {
            id: "mtrial_retry_test".to_string(),
            campaign_id: "mcampaign_retry_test".to_string(),
            plan_digest: "1".repeat(64),
            commit_sha: "a".repeat(40),
            app_version: "0.17.0".to_string(),
            suite_id: "suite".to_string(),
            suite_version: "1.0.0".to_string(),
            suite_digest: "2".repeat(64),
            case_id: "CASE-001".to_string(),
            case_digest: "3".repeat(64),
            trial_index: 0,
            seed: 1,
            model_role: "anchor".to_string(),
            arm: "control".to_string(),
            fault_profile: FaultProfile::Clean,
            orchestration_profile: None,
            model_digest: "4".repeat(64),
            scenario_digest: "5".repeat(64),
            environment_digest: "6".repeat(64),
            verifier_digest: "7".repeat(64),
            prompt_digest: "8".repeat(64),
            tool_schema_digest: "9".repeat(64),
        }
    }

    #[test]
    fn exact_sha_validation_rejects_branch_names() {
        assert!(validate_git_reference("main").is_err());
        assert!(validate_git_reference(&"a".repeat(40)).is_ok());
    }

    #[test]
    fn deterministic_and_model_protocol_versions_are_distinct() {
        assert_ne!(PLAN_SCHEMA_VERSION, ha_eval_spec::PLAN_SCHEMA_VERSION);
        assert_ne!(
            EVIDENCE_SCHEMA_VERSION,
            ha_eval_spec::EVIDENCE_SCHEMA_VERSION
        );
    }

    #[test]
    fn retry_usage_keeps_attempt_trace_graphs_separate() {
        let trial = planned_trial();
        let first = failed_trial_result(
            &trial,
            1,
            "2026-07-18T00:00:00.000Z",
            "2026-07-18T00:00:01.000Z",
            ModelCampaignOutcome::InfraError,
            "provider_unavailable",
            "temporary outage",
            1_000,
        )
        .as_attempt_record();
        let mut recovered = failed_trial_result(
            &trial,
            2,
            "2026-07-18T00:00:02.000Z",
            "2026-07-18T00:00:03.000Z",
            ModelCampaignOutcome::Passed,
            "recovered",
            "",
            1_000,
        );
        let current_trace = recovered.trace.clone();
        merge_retry_usage(&mut recovered, &first).unwrap();

        assert_eq!(recovered.trace, current_trace);
        assert_eq!(recovered.timings.wall_ms, 3_000);
        validate_model_trace(&recovered.trace, &recovered.trace_events, "current").unwrap();
        validate_model_trace(&first.trace, &first.trace_events, "prior").unwrap();
    }

    #[test]
    fn release_identity_rejects_local_dirty_and_wrong_sha_evidence() {
        let reference = "a".repeat(40);
        let runner = model_runner_digest(&reference);
        assert!(validate_release_source_identity(
            &reference,
            ModelCampaignTier::Release,
            &runner,
            false,
            ModelCampaignSource::GithubActions,
            &reference,
            ModelCampaignTier::Release,
            false,
        )
        .is_ok());
        assert!(validate_release_source_identity(
            &reference,
            ModelCampaignTier::Release,
            &runner,
            false,
            ModelCampaignSource::LocalCli,
            &reference,
            ModelCampaignTier::Release,
            false,
        )
        .is_err());
        assert!(validate_release_source_identity(
            &reference,
            ModelCampaignTier::Release,
            &runner,
            true,
            ModelCampaignSource::GithubActions,
            &reference,
            ModelCampaignTier::Release,
            false,
        )
        .is_err());
        assert!(validate_release_source_identity(
            &"b".repeat(40),
            ModelCampaignTier::Release,
            &runner,
            false,
            ModelCampaignSource::GithubActions,
            &reference,
            ModelCampaignTier::Release,
            false,
        )
        .is_err());
    }

    #[test]
    fn evidence_secret_scan_rejects_credentials_but_allows_digests() {
        assert!(scan_evidence_for_secrets(&serde_json::json!({
            "provider": {"apiKey": "canary-secret-value"}
        }))
        .is_err());
        assert!(scan_evidence_for_secrets(&serde_json::json!({
            "error": "Authorization: Bearer canary-secret-value"
        }))
        .is_err());
        assert!(scan_evidence_for_secrets(&serde_json::json!({
            "argumentsDigest": "a".repeat(64),
            "providerId": "eval-anchor"
        }))
        .is_ok());
    }

    #[test]
    fn app_budget_partition_never_expands_small_integer_limits() {
        let budget = CampaignBudget {
            max_model_calls: Some(8),
            max_input_tokens: Some(80),
            max_output_tokens: Some(40),
            max_tool_calls: Some(12),
            max_cost_usd: Some(4.0),
            ..CampaignBudget::default()
        };
        let child = partition_budget(&budget, 4).unwrap();
        assert_eq!(child.max_model_calls, Some(2));
        assert_eq!(child.max_tool_calls, Some(3));
        assert_eq!(child.max_cost_usd, Some(1.0));

        let impossible = CampaignBudget {
            max_model_calls: Some(1),
            ..CampaignBudget::default()
        };
        assert!(partition_budget(&impossible, 2).is_err());
    }
}
