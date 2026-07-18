//! Stable, product-independent protocol for deterministic capability evals.
//!
//! This crate intentionally does not depend on `ha-core`. GitHub release
//! evidence can therefore be inspected and verified without constructing an
//! Agent, loading provider configuration, or touching the product database.

pub mod app;
pub mod model;

use anyhow::{anyhow, bail, Context, Result};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

pub const SUITE_SCHEMA_VERSION: &str = "eval-suite.v1";
pub const POLICY_SCHEMA_VERSION: &str = "eval-policy.v1";
pub const PLAN_SCHEMA_VERSION: &str = "eval-plan.v1";
pub const SHARD_SCHEMA_VERSION: &str = "eval-shard-result.v1";
pub const EVIDENCE_SCHEMA_VERSION: &str = "eval-evidence.v1";
pub const WAIVER_SCHEMA_VERSION: &str = "eval-waiver.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvalAdapter {
    CodingFixturePatch,
    CodingGoldFixturePatch,
    DomainTraceFixture,
    DreamingGolden,
    MemoryRetrievalScale,
}

impl EvalAdapter {
    pub const DETERMINISTIC_V1: [Self; 5] = [
        Self::CodingFixturePatch,
        Self::CodingGoldFixturePatch,
        Self::DomainTraceFixture,
        Self::DreamingGolden,
        Self::MemoryRetrievalScale,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvalTier {
    Weekly,
    Release,
}

impl EvalTier {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Weekly => "weekly",
            Self::Release => "release",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyMode {
    Advisory,
    Enforce,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvalStatus {
    Passed,
    Failed,
    InfraError,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvalCaseSpec {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u64>,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SuiteManifest {
    pub schema_version: String,
    pub id: String,
    pub version: String,
    pub capability: String,
    pub adapter: EvalAdapter,
    pub tiers: Vec<EvalTier>,
    pub runner_class: String,
    pub network_policy: String,
    #[serde(default = "default_shards")]
    pub shards: u16,
    #[serde(default = "default_timeout_seconds")]
    pub timeout_seconds: u64,
    #[serde(default)]
    pub thresholds: BTreeMap<String, Value>,
    pub cases: Vec<EvalCaseSpec>,
}

fn default_shards() -> u16 {
    1
}

fn default_timeout_seconds() -> u64 {
    180
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PolicySuite {
    pub id: String,
    #[serde(default = "default_min_pass_rate")]
    pub min_pass_rate: f64,
}

fn default_min_pass_rate() -> f64 {
    1.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvalPolicy {
    pub schema_version: String,
    pub id: String,
    pub version: String,
    pub tier: EvalTier,
    pub mode: PolicyMode,
    pub allowed_adapters: Vec<EvalAdapter>,
    pub suites: Vec<PolicySuite>,
    #[serde(default)]
    pub performance_blocking: bool,
    #[serde(default = "default_max_duration_seconds")]
    pub max_duration_seconds: u64,
}

fn default_max_duration_seconds() -> u64 {
    1_800
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlannedCase {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub digest: String,
    pub timeout_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlannedSuite {
    pub id: String,
    pub version: String,
    pub capability: String,
    pub adapter: EvalAdapter,
    pub digest: String,
    pub shards: u16,
    pub cases: Vec<PlannedCase>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvalPlan {
    pub schema_version: String,
    pub reference: String,
    pub tier: EvalTier,
    pub policy_id: String,
    pub policy_version: String,
    pub policy_digest: String,
    pub runner_digest: String,
    pub suites: Vec<PlannedSuite>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvalCheck {
    pub name: String,
    pub status: EvalStatus,
    pub detail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metric: Option<f64>,
    #[serde(default)]
    pub advisory: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaseResult {
    pub suite_id: String,
    pub case_id: String,
    pub case_digest: String,
    pub status: EvalStatus,
    pub duration_ms: u64,
    pub attempt: u8,
    pub checks: Vec<EvalCheck>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShardResult {
    pub schema_version: String,
    pub reference: String,
    pub runner_digest: String,
    pub suite_id: String,
    pub suite_digest: String,
    pub shard_index: u16,
    pub shard_total: u16,
    pub started_at: String,
    pub completed_at: String,
    pub duration_ms: u64,
    pub cases: Vec<CaseResult>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactDigest {
    pub path: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvalWaiver {
    pub schema_version: String,
    pub commit_sha: String,
    pub tag: String,
    pub reason: String,
    pub suites: Vec<String>,
    pub approved_by: String,
    pub approved_at: String,
    pub workflow_run_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvalEvidence {
    pub schema_version: String,
    pub commit_sha: String,
    pub dirty: bool,
    pub source: String,
    pub app_version: String,
    pub tier: EvalTier,
    pub policy_id: String,
    pub policy_version: String,
    pub policy_mode: PolicyMode,
    pub policy_digest: String,
    pub runner_digest: String,
    pub aggregate_status: EvalStatus,
    pub started_at: String,
    pub completed_at: String,
    pub duration_ms: u64,
    pub suites: Vec<PlannedSuite>,
    pub cases: Vec<CaseResult>,
    pub artifacts: Vec<ArtifactDigest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiver: Option<EvalWaiver>,
}

pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let raw =
        fs::read_to_string(path).with_context(|| format!("reading JSON {}", path.display()))?;
    serde_json::from_str(&raw).with_context(|| format!("parsing JSON {}", path.display()))
}

pub fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating directory {}", parent.display()))?;
    }
    let bytes = serde_json::to_vec_pretty(value)?;
    fs::write(path, bytes).with_context(|| format!("writing JSON {}", path.display()))
}

pub fn canonical_json(value: &Value) -> Result<Vec<u8>> {
    serde_json::to_vec(&canonical_value(value)).context("serializing canonical JSON")
}

pub fn canonical_value(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let sorted = object
                .iter()
                .map(|(key, value)| (key.clone(), canonical_value(value)))
                .collect::<BTreeMap<_, _>>();
            Value::Object(sorted.into_iter().collect())
        }
        Value::Array(items) => Value::Array(items.iter().map(canonical_value).collect()),
        Value::Number(number) => number
            .is_f64()
            .then(|| number.as_f64())
            .flatten()
            .filter(|value| value.is_finite() && value.fract() == 0.0)
            .and_then(|value| {
                if value >= 0.0 && value <= u64::MAX as f64 {
                    Some(Value::Number(serde_json::Number::from(value as u64)))
                } else if value >= i64::MIN as f64 && value <= i64::MAX as f64 {
                    Some(Value::Number(serde_json::Number::from(value as i64)))
                } else {
                    None
                }
            })
            .unwrap_or_else(|| value.clone()),
        other => other.clone(),
    }
}

pub fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn digest_serializable<T: Serialize>(value: &T) -> Result<String> {
    let value = serde_json::to_value(value)?;
    Ok(sha256_bytes(&canonical_json(&value)?))
}

pub fn digest_file(path: &Path) -> Result<String> {
    let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(sha256_bytes(&bytes))
}

pub fn resolve_contained(base: &Path, relative: &str) -> Result<PathBuf> {
    let relative_path = Path::new(relative);
    if relative.trim().is_empty() || relative_path.is_absolute() {
        bail!("eval asset path must be a non-empty relative path: {relative:?}");
    }
    if relative_path
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        bail!("eval asset path may not contain parent/root components: {relative:?}");
    }
    let canonical_base = base
        .canonicalize()
        .with_context(|| format!("canonicalizing eval suite directory {}", base.display()))?;
    let joined = canonical_base.join(relative_path);
    let canonical_joined = joined
        .canonicalize()
        .with_context(|| format!("canonicalizing eval asset {}", joined.display()))?;
    if !canonical_joined.starts_with(&canonical_base) {
        bail!(
            "eval asset escaped suite directory: {}",
            canonical_joined.display()
        );
    }
    Ok(canonical_joined)
}

pub fn validate_suite(manifest: &SuiteManifest, suite_dir: &Path) -> Result<()> {
    if manifest.schema_version != SUITE_SCHEMA_VERSION {
        bail!("suite {} has unsupported schemaVersion", manifest.id);
    }
    validate_identifier("suite id", &manifest.id)?;
    validate_identifier("capability", &manifest.capability)?;
    if manifest.version.trim().is_empty() {
        bail!("suite {} has an empty version", manifest.id);
    }
    if !EvalAdapter::DETERMINISTIC_V1.contains(&manifest.adapter) {
        bail!("suite {} uses a non-deterministic adapter", manifest.id);
    }
    if manifest.runner_class != "hosted_linux" {
        bail!(
            "suite {} runnerClass must be hosted_linux in v1",
            manifest.id
        );
    }
    if manifest.network_policy != "deny" {
        bail!("suite {} networkPolicy must be deny in v1", manifest.id);
    }
    if !(1..=64).contains(&manifest.shards) {
        bail!("suite {} shards must be between 1 and 64", manifest.id);
    }
    if !(1..=900).contains(&manifest.timeout_seconds) {
        bail!(
            "suite {} timeoutSeconds must be between 1 and 900",
            manifest.id
        );
    }
    if manifest.tiers.is_empty() || manifest.cases.is_empty() {
        bail!("suite {} must declare tiers and cases", manifest.id);
    }
    let mut ids = BTreeSet::new();
    for case in &manifest.cases {
        validate_identifier("case id", &case.id)?;
        if !ids.insert(case.id.clone()) {
            bail!("suite {} contains duplicate case {}", manifest.id, case.id);
        }
        if case
            .timeout_seconds
            .is_some_and(|value| !(1..=900).contains(&value))
        {
            bail!(
                "suite {} case {} timeout must be 1..=900",
                manifest.id,
                case.id
            );
        }
        if let Some(path) = &case.path {
            resolve_contained(suite_dir, path)?;
        }
    }
    Ok(())
}

pub fn validate_policy(policy: &EvalPolicy) -> Result<()> {
    if policy.schema_version != POLICY_SCHEMA_VERSION {
        bail!("policy {} has unsupported schemaVersion", policy.id);
    }
    validate_identifier("policy id", &policy.id)?;
    if policy.version.trim().is_empty() || policy.suites.is_empty() {
        bail!("policy {} must declare version and suites", policy.id);
    }
    if policy.performance_blocking {
        bail!("v1 performance metrics must remain advisory");
    }
    if policy.max_duration_seconds == 0 {
        bail!("policy maxDurationSeconds must be positive");
    }
    let allowed = policy
        .allowed_adapters
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    if allowed.is_empty()
        || allowed
            .iter()
            .any(|adapter| !EvalAdapter::DETERMINISTIC_V1.contains(adapter))
    {
        bail!("policy {} allows an unsupported adapter", policy.id);
    }
    let mut suites = BTreeSet::new();
    for suite in &policy.suites {
        validate_identifier("policy suite id", &suite.id)?;
        if !(0.0..=1.0).contains(&suite.min_pass_rate) {
            bail!("policy suite {} minPassRate must be 0..=1", suite.id);
        }
        if !suites.insert(&suite.id) {
            bail!("policy {} contains duplicate suite {}", policy.id, suite.id);
        }
    }
    Ok(())
}

fn validate_identifier(label: &str, value: &str) -> Result<()> {
    if value.is_empty()
        || !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
    {
        bail!("{label} must contain only ASCII letters, numbers, '-' or '_': {value:?}");
    }
    Ok(())
}

pub fn case_digest(case: &EvalCaseSpec, suite_dir: &Path) -> Result<String> {
    let mut value = serde_json::to_value(case)?;
    if let Some(path) = &case.path {
        let path = resolve_contained(suite_dir, path)?;
        value
            .as_object_mut()
            .ok_or_else(|| anyhow!("case did not serialize as an object"))?
            .insert(
                "assetSha256".to_string(),
                Value::String(digest_file(&path)?),
            );
    }
    Ok(sha256_bytes(&canonical_json(&value)?))
}

pub fn suite_digest(manifest: &SuiteManifest, suite_dir: &Path) -> Result<String> {
    let mut value = serde_json::to_value(manifest)?;
    let assets = manifest
        .cases
        .iter()
        .map(|case| {
            Ok((
                case.id.clone(),
                Value::String(case_digest(case, suite_dir)?),
            ))
        })
        .collect::<Result<Map<String, Value>>>()?;
    value
        .as_object_mut()
        .ok_or_else(|| anyhow!("suite did not serialize as an object"))?
        .insert("caseDigests".to_string(), Value::Object(assets));
    Ok(sha256_bytes(&canonical_json(&value)?))
}

pub fn stable_shard(id: &str, total: u16) -> u16 {
    let hash = Sha256::digest(id.as_bytes());
    let number = u64::from_be_bytes(hash[..8].try_into().expect("SHA-256 has eight bytes"));
    (number % u64::from(total.max(1))) as u16
}

/// Validate the JSON Schema subset used by the committed v1 schemas.
///
/// Remote `$ref`, regex patterns, coercion, and defaults are intentionally not
/// supported. The committed schemas only use type/required/properties/items,
/// enum/const, numeric bounds and additionalProperties=false.
pub fn validate_json_schema(instance: &Value, schema: &Value) -> Result<()> {
    validate_schema_node(instance, schema, "$")
}

fn validate_schema_node(instance: &Value, schema: &Value, location: &str) -> Result<()> {
    if let Some(expected) = schema.get("type").and_then(Value::as_str) {
        let matches = match expected {
            "object" => instance.is_object(),
            "array" => instance.is_array(),
            "string" => instance.is_string(),
            "number" => instance.is_number(),
            "integer" => instance.as_i64().is_some() || instance.as_u64().is_some(),
            "boolean" => instance.is_boolean(),
            "null" => instance.is_null(),
            other => bail!("unsupported JSON Schema type {other:?}"),
        };
        if !matches {
            bail!("{location}: expected JSON type {expected}");
        }
    }
    if let Some(constant) = schema.get("const") {
        if instance != constant {
            bail!("{location}: value does not match const");
        }
    }
    if let Some(values) = schema.get("enum").and_then(Value::as_array) {
        if !values.contains(instance) {
            bail!("{location}: value is not in enum");
        }
    }
    if let Some(object) = instance.as_object() {
        let properties = schema
            .get("properties")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        if let Some(required) = schema.get("required").and_then(Value::as_array) {
            for key in required.iter().filter_map(Value::as_str) {
                if !object.contains_key(key) {
                    bail!("{location}: missing required property {key}");
                }
            }
        }
        if schema.get("additionalProperties") == Some(&Value::Bool(false)) {
            for key in object.keys() {
                if !properties.contains_key(key) {
                    bail!("{location}: unknown property {key}");
                }
            }
        }
        for (key, child_schema) in properties {
            if let Some(child) = object.get(&key) {
                validate_schema_node(child, &child_schema, &format!("{location}.{key}"))?;
            }
        }
    }
    if let Some(items) = instance.as_array() {
        if let Some(item_schema) = schema.get("items") {
            for (index, item) in items.iter().enumerate() {
                validate_schema_node(item, item_schema, &format!("{location}[{index}]"))?;
            }
        }
        if let Some(min) = schema.get("minItems").and_then(Value::as_u64) {
            if items.len() < min as usize {
                bail!("{location}: expected at least {min} items");
            }
        }
    }
    if let Some(number) = instance.as_f64() {
        if schema
            .get("minimum")
            .and_then(Value::as_f64)
            .is_some_and(|min| number < min)
        {
            bail!("{location}: number is below minimum");
        }
        if schema
            .get("maximum")
            .and_then(Value::as_f64)
            .is_some_and(|max| number > max)
        {
            bail!("{location}: number is above maximum");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_digest_ignores_object_key_order() {
        let a = serde_json::json!({"b": 2, "a": {"d": 4, "c": 3}});
        let b = serde_json::json!({"a": {"c": 3, "d": 4}, "b": 2});
        assert_eq!(canonical_json(&a).unwrap(), canonical_json(&b).unwrap());
    }

    #[test]
    fn canonical_digest_normalizes_equivalent_integer_numbers() {
        let integer = serde_json::json!({"rate": 1});
        let float = serde_json::json!({"rate": 1.0});
        assert_eq!(
            canonical_json(&integer).unwrap(),
            canonical_json(&float).unwrap()
        );
    }

    #[test]
    fn stable_shard_is_bounded_and_stable() {
        assert_eq!(stable_shard("CE-BUG-001", 4), stable_shard("CE-BUG-001", 4));
        assert!(stable_shard("CE-BUG-001", 4) < 4);
    }

    #[test]
    fn paths_reject_parent_components_before_touching_disk() {
        let error = resolve_contained(Path::new("."), "../secret").unwrap_err();
        assert!(error.to_string().contains("parent/root"));
    }
}
