//! Issue reporting configuration (`AppConfig.issue_reporting`).

use serde::{Deserialize, Serialize};

pub const DEFAULT_ISSUE_OWNER: &str = "shiwenwen";
pub const DEFAULT_ISSUE_REPO: &str = "hope-agent";
pub const DEFAULT_GITHUB_API_BASE_URL: &str = "https://api.github.com";
const DEFAULT_MAX_EVIDENCE_CHARS: usize = 24_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IssueReportingConfig {
    #[serde(default = "crate::default_true")]
    pub enabled: bool,
    #[serde(default = "default_owner")]
    pub owner: String,
    #[serde(default = "default_repo")]
    pub repo: String,
    #[serde(default = "default_api_base_url")]
    pub api_base_url: String,
    #[serde(default)]
    pub labels_by_kind: IssueLabelsByKind,
    #[serde(default = "default_max_evidence_chars")]
    pub max_evidence_chars: usize,
    #[serde(default = "crate::default_true")]
    pub duplicate_check_enabled: bool,
}

fn default_owner() -> String {
    DEFAULT_ISSUE_OWNER.to_string()
}

fn default_repo() -> String {
    DEFAULT_ISSUE_REPO.to_string()
}

fn default_api_base_url() -> String {
    DEFAULT_GITHUB_API_BASE_URL.to_string()
}

fn default_max_evidence_chars() -> usize {
    DEFAULT_MAX_EVIDENCE_CHARS
}

impl Default for IssueReportingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            owner: default_owner(),
            repo: default_repo(),
            api_base_url: default_api_base_url(),
            labels_by_kind: IssueLabelsByKind::default(),
            max_evidence_chars: DEFAULT_MAX_EVIDENCE_CHARS,
            duplicate_check_enabled: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IssueLabelsByKind {
    #[serde(default = "default_bug_labels")]
    pub bug: Vec<String>,
    #[serde(default = "default_feature_labels")]
    pub feature: Vec<String>,
    #[serde(default = "default_improvement_labels")]
    pub improvement: Vec<String>,
}

fn default_bug_labels() -> Vec<String> {
    vec!["bug".to_string()]
}

fn default_feature_labels() -> Vec<String> {
    vec!["enhancement".to_string()]
}

fn default_improvement_labels() -> Vec<String> {
    vec!["improvement".to_string()]
}

impl Default for IssueLabelsByKind {
    fn default() -> Self {
        Self {
            bug: default_bug_labels(),
            feature: default_feature_labels(),
            improvement: default_improvement_labels(),
        }
    }
}
