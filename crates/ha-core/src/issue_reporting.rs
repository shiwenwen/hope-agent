use anyhow::{bail, Context, Result};
use once_cell::sync::Lazy;
use regex::Regex;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, USER_AGENT};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncWriteExt;

pub const DEFAULT_ISSUE_OWNER: &str = "shiwenwen";
pub const DEFAULT_ISSUE_REPO: &str = "hope-agent";
pub const DEFAULT_GITHUB_API_BASE_URL: &str = "https://api.github.com";
const DEFAULT_MAX_EVIDENCE_CHARS: usize = 24_000;
const MAX_GITHUB_ERROR_CHARS: usize = 2_000;
const GH_TIMEOUT_SECS: u64 = 30;

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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IssueKind {
    Bug,
    Feature,
    Improvement,
}

impl IssueKind {
    pub fn labels<'a>(&self, cfg: &'a IssueReportingConfig) -> &'a [String] {
        match self {
            Self::Bug => &cfg.labels_by_kind.bug,
            Self::Feature => &cfg.labels_by_kind.feature,
            Self::Improvement => &cfg.labels_by_kind.improvement,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bug => "bug",
            Self::Feature => "feature",
            Self::Improvement => "improvement",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueReportingConfigStatus {
    pub config: IssueReportingConfig,
    pub has_token: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueDraft {
    pub owner: String,
    pub repo: String,
    pub kind: IssueKind,
    pub title: String,
    pub body: String,
    pub labels: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueSummary {
    pub number: u64,
    pub title: String,
    pub state: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatedIssue {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub created_via: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_warning: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueReportingTestResult {
    pub owner: String,
    pub repo: String,
    pub has_token: bool,
    pub backend: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GitHubIssueCredential {
    token: String,
}

pub fn get_config_status() -> IssueReportingConfigStatus {
    IssueReportingConfigStatus {
        config: crate::config::cached_config().issue_reporting.clone(),
        has_token: has_token(),
    }
}

pub fn save_token(token: Option<String>) -> Result<()> {
    let path = credential_path()?;
    let token = token.unwrap_or_default();
    let token = token.trim();
    if token.is_empty() {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).with_context(|| format!("remove {}", path.display())),
        }
        return Ok(());
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(&GitHubIssueCredential {
        token: token.to_string(),
    })?;
    crate::platform::write_secure_file(&path, &bytes)
        .with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

pub fn has_token() -> bool {
    read_token().ok().flatten().is_some()
}

fn credential_path() -> Result<PathBuf> {
    crate::paths::github_issue_credential_path()
}

fn read_token() -> Result<Option<String>> {
    let path = credential_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let cred: GitHubIssueCredential =
        serde_json::from_str(&raw).with_context(|| format!("parse {}", path.display()))?;
    let token = cred.token.trim();
    if token.is_empty() {
        Ok(None)
    } else {
        Ok(Some(token.to_string()))
    }
}

pub fn sanitize_issue_text(input: &str, max_chars: usize) -> String {
    let redacted = redact_issue_plaintext_secrets(input);
    let cap = max_chars.max(1);
    if redacted.len() <= cap {
        return redacted;
    }
    format!(
        "{}\n\n[truncated to {} bytes by Hope Agent]",
        crate::truncate_utf8(&redacted, cap),
        cap
    )
}

fn redact_issue_plaintext_secrets(input: &str) -> String {
    static AUTH_HEADER_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r#"(?i)\b(authorization\s*:\s*(?:bearer|token|basic)\s+)[^\s,'")`]+"#)
            .expect("valid auth header regex")
    });
    static BEARER_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r#"(?i)\bbearer\s+[A-Za-z0-9._~+/=-]{8,}"#).expect("valid bearer regex")
    });
    static TOKEN_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(
            r"(?:sk|pat)-[A-Za-z0-9_-]{20,}|github_pat_[A-Za-z0-9_]{20,}|gh[pousr]_[A-Za-z0-9_]{20,}|AIza[A-Za-z0-9_-]{35}|xox[abp]-[A-Za-z0-9_-]{20,}",
        )
        .expect("valid token regex")
    });

    let mut out = crate::logging::redact_sensitive(input);
    out = AUTH_HEADER_RE
        .replace_all(&out, "${1}[REDACTED]")
        .into_owned();
    out = BEARER_RE
        .replace_all(&out, "Bearer [REDACTED]")
        .into_owned();
    TOKEN_RE.replace_all(&out, "[REDACTED]").into_owned()
}

pub fn normalize_draft(
    cfg: &IssueReportingConfig,
    kind: IssueKind,
    title: &str,
    body: &str,
    labels: Option<Vec<String>>,
) -> Result<IssueDraft> {
    validate_repo_config(cfg)?;
    let title = title.trim();
    if title.is_empty() {
        bail!("issue title must not be empty");
    }
    let mut final_labels = labels.unwrap_or_else(|| kind.labels(cfg).to_vec());
    final_labels.retain(|s| !s.trim().is_empty());
    final_labels.sort();
    final_labels.dedup();
    Ok(IssueDraft {
        owner: cfg.owner.trim().to_string(),
        repo: cfg.repo.trim().to_string(),
        kind,
        title: sanitize_issue_title(title),
        body: sanitize_issue_text(body, cfg.max_evidence_chars),
        labels: final_labels,
    })
}

pub async fn search_issues(cfg: &IssueReportingConfig, query: &str) -> Result<Vec<IssueSummary>> {
    validate_repo_config(cfg)?;
    let q = query.trim();
    if q.is_empty() {
        bail!("search query must not be empty");
    }
    if read_token()?.is_none() && gh_available() {
        if let Ok(items) = gh_search_issues(cfg, q).await {
            return Ok(items);
        }
    }

    let search = format!(
        "repo:{}/{} is:issue state:open {}",
        cfg.owner.trim(),
        cfg.repo.trim(),
        q
    );
    let encoded = urlencoding::encode(&search);
    let url = github_url(cfg, &format!("/search/issues?q={encoded}&per_page=10"))?;
    ssrf_check(&url).await?;
    let client = github_client()?;
    let req = client
        .get(&url)
        .headers(github_headers(read_token()?.as_deref())?);
    let resp = req.send().await.with_context(|| format!("GET {url}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!(
            "GitHub issue search failed with HTTP {}: {}",
            status,
            sanitize_issue_text(&text, MAX_GITHUB_ERROR_CHARS)
        );
    }
    let parsed: SearchResponse = serde_json::from_str(&text)?;
    Ok(parsed
        .items
        .into_iter()
        .map(|item| IssueSummary {
            number: item.number,
            title: item.title,
            state: item.state,
            url: item.html_url,
        })
        .collect())
}

pub async fn test_connection(cfg: &IssueReportingConfig) -> Result<IssueReportingTestResult> {
    validate_repo_config(cfg)?;
    let Some(token) = read_token()? else {
        return test_gh_connection(cfg).await;
    };
    let path = format!("/repos/{}/{}", cfg.owner.trim(), cfg.repo.trim());
    let url = github_url(cfg, &path)?;
    ssrf_check(&url).await?;
    let client = github_client()?;
    let resp = client
        .get(&url)
        .headers(github_headers(Some(&token))?)
        .send()
        .await
        .with_context(|| format!("GET {url}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!(
            "GitHub repo probe failed with HTTP {}: {}",
            status,
            sanitize_issue_text(&text, MAX_GITHUB_ERROR_CHARS)
        );
    }
    Ok(IssueReportingTestResult {
        owner: cfg.owner.trim().to_string(),
        repo: cfg.repo.trim().to_string(),
        has_token: true,
        backend: "github-rest-token".to_string(),
        message: "GitHub token can reach the configured repository.".to_string(),
    })
}

pub async fn create_issue(cfg: &IssueReportingConfig, draft: &IssueDraft) -> Result<CreatedIssue> {
    if !cfg.enabled {
        bail!("Issue reporting is disabled in settings");
    }
    validate_repo_config(cfg)?;
    let Some(token) = read_token()? else {
        return gh_create_issue(cfg, draft).await;
    };
    let path = format!("/repos/{}/{}/issues", draft.owner, draft.repo);
    let url = github_url(cfg, &path)?;
    ssrf_check(&url).await?;
    let client = github_client()?;
    let headers = github_headers(Some(&token))?;

    let first = post_issue(&client, &url, headers.clone(), draft, true).await?;
    if first.status.is_success() {
        return parse_created_issue(first.text, None);
    }

    if first.status.as_u16() == 422 && !draft.labels.is_empty() {
        let retry = post_issue(&client, &url, headers, draft, false).await?;
        if retry.status.is_success() {
            return parse_created_issue(
                retry.text,
                Some(
                    "Configured labels were rejected by GitHub; created the issue without labels."
                        .to_string(),
                ),
            );
        }
        return Err(github_error(
            "GitHub create issue failed",
            retry.status,
            &retry.text,
        ));
    }

    Err(github_error(
        "GitHub create issue failed",
        first.status,
        &first.text,
    ))
}

fn parse_created_issue(text: String, label_warning: Option<String>) -> Result<CreatedIssue> {
    let parsed: CreatedIssueResponse = serde_json::from_str(&text)?;
    Ok(CreatedIssue {
        number: parsed.number,
        title: parsed.title,
        url: parsed.html_url,
        created_via: "github-rest-token".to_string(),
        label_warning,
    })
}

struct PostIssueResult {
    status: reqwest::StatusCode,
    text: String,
}

async fn post_issue(
    client: &reqwest::Client,
    url: &str,
    headers: HeaderMap,
    draft: &IssueDraft,
    include_labels: bool,
) -> Result<PostIssueResult> {
    let mut payload = json!({
        "title": draft.title,
        "body": draft.body,
    });
    if include_labels && !draft.labels.is_empty() {
        payload["labels"] = json!(draft.labels);
    }
    let resp = client
        .post(url)
        .headers(headers)
        .json(&payload)
        .send()
        .await
        .with_context(|| format!("POST {url}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    Ok(PostIssueResult { status, text })
}

fn github_error(prefix: &str, status: reqwest::StatusCode, text: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "{} with HTTP {}: {}",
        prefix,
        status,
        sanitize_issue_text(text, MAX_GITHUB_ERROR_CHARS)
    )
}

async fn test_gh_connection(cfg: &IssueReportingConfig) -> Result<IssueReportingTestResult> {
    ssrf_check(&github_url(
        cfg,
        &format!("/repos/{}/{}", cfg.owner, cfg.repo),
    )?)
    .await?;
    ensure_gh_authenticated(cfg).await?;
    let repo = gh_repo_arg(cfg)?;
    let output = run_gh(
        vec![
            "repo".to_string(),
            "view".to_string(),
            repo,
            "--json".to_string(),
            "nameWithOwner,url".to_string(),
        ],
        None,
    )
    .await?;
    if !output.status.success() {
        bail!(
            "gh could not reach the configured repository: {}",
            command_output_error(&output)
        );
    }
    Ok(IssueReportingTestResult {
        owner: cfg.owner.trim().to_string(),
        repo: cfg.repo.trim().to_string(),
        has_token: false,
        backend: "gh-cli".to_string(),
        message:
            "No GitHub token is configured; gh CLI is authenticated and can reach the repository."
                .to_string(),
    })
}

async fn gh_search_issues(cfg: &IssueReportingConfig, query: &str) -> Result<Vec<IssueSummary>> {
    ssrf_check(&github_url(
        cfg,
        &format!("/repos/{}/{}", cfg.owner, cfg.repo),
    )?)
    .await?;
    ensure_gh_authenticated(cfg).await?;
    let output = run_gh(
        vec![
            "issue".to_string(),
            "list".to_string(),
            "--repo".to_string(),
            gh_repo_arg(cfg)?,
            "--state".to_string(),
            "open".to_string(),
            "--search".to_string(),
            query.to_string(),
            "--limit".to_string(),
            "10".to_string(),
            "--json".to_string(),
            "number,title,state,url".to_string(),
        ],
        None,
    )
    .await?;
    if !output.status.success() {
        bail!("gh issue search failed: {}", command_output_error(&output));
    }
    let parsed: Vec<GhIssueItem> = serde_json::from_slice(&output.stdout)?;
    Ok(parsed
        .into_iter()
        .map(|item| IssueSummary {
            number: item.number,
            title: item.title,
            state: item.state,
            url: item.url,
        })
        .collect())
}

async fn gh_create_issue(cfg: &IssueReportingConfig, draft: &IssueDraft) -> Result<CreatedIssue> {
    if !gh_available() {
        bail!(
            "GitHub token is not configured and `gh` CLI was not found. Configure a token in Settings or install GitHub CLI and run `gh auth login`."
        );
    }
    ssrf_check(&github_url(
        cfg,
        &format!("/repos/{}/{}", draft.owner, draft.repo),
    )?)
    .await?;
    ensure_gh_authenticated(cfg).await?;
    let first = gh_issue_create_once(cfg, draft, true).await;
    match first {
        Ok(created) => Ok(created),
        Err(first_err) if !draft.labels.is_empty() && is_likely_label_error(&first_err) => {
            let retry = gh_issue_create_once(cfg, draft, false).await;
            match retry {
                Ok(mut created) => {
                    created.label_warning = Some(
                        "Configured labels were rejected by gh/GitHub; created the issue without labels."
                            .to_string(),
                    );
                    Ok(created)
                }
                Err(_) => Err(first_err),
            }
        }
        Err(e) => Err(e),
    }
}

fn is_likely_label_error(err: &anyhow::Error) -> bool {
    let text = format!("{:#}", err).to_ascii_lowercase();
    text.contains("label")
        && (text.contains("not found")
            || text.contains("could not add")
            || text.contains("invalid")
            || text.contains("does not exist"))
}

async fn gh_issue_create_once(
    cfg: &IssueReportingConfig,
    draft: &IssueDraft,
    include_labels: bool,
) -> Result<CreatedIssue> {
    let mut args = vec![
        "issue".to_string(),
        "create".to_string(),
        "--repo".to_string(),
        gh_repo_arg(cfg)?,
        "--title".to_string(),
        draft.title.clone(),
        "--body-file".to_string(),
        "-".to_string(),
    ];
    if include_labels {
        for label in &draft.labels {
            args.push("--label".to_string());
            args.push(label.clone());
        }
    }
    let output = run_gh(args, Some(draft.body.clone())).await?;
    if !output.status.success() {
        bail!("gh issue create failed: {}", command_output_error(&output));
    }
    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if url.is_empty() {
        bail!("gh issue create succeeded but did not return an issue URL");
    }
    Ok(CreatedIssue {
        number: parse_issue_number_from_url(&url).unwrap_or(0),
        title: draft.title.clone(),
        url,
        created_via: "gh-cli".to_string(),
        label_warning: None,
    })
}

async fn ensure_gh_authenticated(cfg: &IssueReportingConfig) -> Result<()> {
    if !gh_available() {
        bail!(
            "`gh` CLI was not found. Install GitHub CLI and run `gh auth login`, or configure a GitHub token in Settings."
        );
    }
    let output = run_gh(
        vec![
            "auth".to_string(),
            "status".to_string(),
            "--hostname".to_string(),
            gh_hostname(cfg)?,
        ],
        None,
    )
    .await?;
    if !output.status.success() {
        bail!(
            "`gh` CLI is not authenticated for {}. Run `gh auth login --hostname {}` or configure a GitHub token in Settings. {}",
            gh_hostname(cfg)?,
            gh_hostname(cfg)?,
            command_output_error(&output)
        );
    }
    Ok(())
}

async fn run_gh(args: Vec<String>, stdin: Option<String>) -> Result<std::process::Output> {
    let gh = which::which("gh").context("`gh` CLI was not found on PATH")?;
    let mut cmd = tokio::process::Command::new(gh);
    cmd.args(args)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().context("failed to spawn gh CLI")?;
    if let Some(input) = stdin {
        let mut child_stdin = child.stdin.take().context("failed to open gh stdin")?;
        child_stdin
            .write_all(input.as_bytes())
            .await
            .context("failed to write gh stdin")?;
        drop(child_stdin);
    }
    tokio::time::timeout(
        Duration::from_secs(GH_TIMEOUT_SECS),
        child.wait_with_output(),
    )
    .await
    .context("gh CLI timed out")?
    .context("failed to wait for gh CLI")
}

fn gh_available() -> bool {
    which::which("gh").is_ok()
}

fn command_output_error(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let combined = if stderr.trim().is_empty() {
        stdout.trim().to_string()
    } else if stdout.trim().is_empty() {
        stderr.trim().to_string()
    } else {
        format!("{}\n{}", stderr.trim(), stdout.trim())
    };
    sanitize_issue_text(&combined, MAX_GITHUB_ERROR_CHARS)
}

fn gh_repo_arg(cfg: &IssueReportingConfig) -> Result<String> {
    let host = if is_default_github_api(&cfg.api_base_url) {
        None
    } else {
        Some(gh_hostname(cfg)?)
    };
    gh_repo_arg_for_parts(&cfg.owner, &cfg.repo, host.as_deref())
}

fn gh_repo_arg_for_parts(owner: &str, repo: &str, host: Option<&str>) -> Result<String> {
    validate_repo_part("owner", owner)?;
    validate_repo_part("repo", repo)?;
    Ok(match host {
        Some(host) => format!("{}/{}/{}", host, owner.trim(), repo.trim()),
        None => format!("{}/{}", owner.trim(), repo.trim()),
    })
}

fn gh_hostname(cfg: &IssueReportingConfig) -> Result<String> {
    if is_default_github_api(&cfg.api_base_url) {
        return Ok("github.com".to_string());
    }
    let parsed = url::Url::parse(cfg.api_base_url.trim())
        .with_context(|| format!("invalid apiBaseUrl: {}", cfg.api_base_url))?;
    parsed
        .host_str()
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("apiBaseUrl has no hostname"))
}

fn is_default_github_api(api_base_url: &str) -> bool {
    api_base_url.trim().trim_end_matches('/') == DEFAULT_GITHUB_API_BASE_URL
}

fn github_client() -> Result<reqwest::Client> {
    let builder = reqwest::Client::builder().timeout(Duration::from_secs(30));
    crate::provider::apply_proxy(builder)
        .build()
        .context("reqwest client build failed")
}

fn github_headers(token: Option<&str>) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    headers.insert(
        ACCEPT,
        HeaderValue::from_static("application/vnd.github+json"),
    );
    headers.insert(
        "X-GitHub-Api-Version",
        HeaderValue::from_static("2022-11-28"),
    );
    headers.insert(
        USER_AGENT,
        HeaderValue::from_str(&format!("Hope-Agent/{}", crate::app_version()))?,
    );
    if let Some(token) = token {
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}"))?,
        );
    }
    Ok(headers)
}

fn github_url(cfg: &IssueReportingConfig, path_and_query: &str) -> Result<String> {
    let base = cfg.api_base_url.trim().trim_end_matches('/');
    if base.is_empty() {
        bail!("apiBaseUrl must not be empty");
    }
    let url = format!("{base}{path_and_query}");
    url::Url::parse(&url).with_context(|| format!("invalid GitHub API URL: {url}"))?;
    Ok(url)
}

async fn ssrf_check(url: &str) -> Result<()> {
    let ssrf_cfg = &crate::config::cached_config().ssrf;
    crate::security::ssrf::check_url(
        url,
        crate::security::ssrf::SsrfPolicy::Default,
        &ssrf_cfg.trusted_hosts,
    )
    .await
    .with_context(|| format!("SSRF check failed for {url}"))?;
    Ok(())
}

fn validate_repo_config(cfg: &IssueReportingConfig) -> Result<()> {
    validate_repo_part("owner", &cfg.owner)?;
    validate_repo_part("repo", &cfg.repo)?;
    url::Url::parse(cfg.api_base_url.trim())
        .with_context(|| format!("invalid apiBaseUrl: {}", cfg.api_base_url))?;
    Ok(())
}

fn validate_repo_part(name: &str, value: &str) -> Result<()> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!("{name} must not be empty");
    }
    if trimmed.len() > 100 {
        bail!("{name} is too long");
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        bail!("{name} contains unsupported characters");
    }
    Ok(())
}

fn sanitize_issue_title(input: &str) -> String {
    let redacted = redact_issue_plaintext_secrets(input);
    let one_line = redacted.split_whitespace().collect::<Vec<_>>().join(" ");
    crate::truncate_utf8(&one_line, 256).trim().to_string()
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    #[serde(default)]
    items: Vec<SearchIssueItem>,
}

#[derive(Debug, Deserialize)]
struct SearchIssueItem {
    number: u64,
    title: String,
    state: String,
    html_url: String,
}

#[derive(Debug, Deserialize)]
struct CreatedIssueResponse {
    number: u64,
    title: String,
    html_url: String,
}

#[derive(Debug, Deserialize)]
struct GhIssueItem {
    number: u64,
    title: String,
    state: String,
    url: String,
}

fn parse_issue_number_from_url(url: &str) -> Option<u64> {
    url.trim_end_matches('/')
        .rsplit('/')
        .next()
        .and_then(|s| s.parse::<u64>().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_targets_main_repo() {
        let cfg = IssueReportingConfig::default();
        assert_eq!(cfg.owner, "shiwenwen");
        assert_eq!(cfg.repo, "hope-agent");
        assert!(cfg.duplicate_check_enabled);
    }

    #[test]
    fn sanitize_redacts_and_truncates() {
        let raw = r#"{"apiKey":"secret","message":"abcdefghijklmnopqrstuvwxyz"}"#;
        let out = sanitize_issue_text(raw, 32);
        assert!(out.contains("[REDACTED]"));
        assert!(out.contains("truncated"));
        assert!(!out.contains("secret"));
    }

    #[test]
    fn sanitize_scrubs_plaintext_tokens_before_truncating() {
        let bearer = "Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9";
        let openai = "sk-proj-abcdefghijklmnopqrstuvwxyz123456";
        let github = "github_pat_1234567890ABCDEFGHIJKLMNOPQRSTUVWXYZ_abcdefghi";
        let classic = "ghp_1234567890ABCDEFGHIJKLMNOPQRSTUVWXYZ";
        let raw = format!("{bearer}\nopenai={openai}\ngithub={github}\nclassic={classic}");
        let out = sanitize_issue_text(&raw, 512);

        assert!(!out.contains("eyJhbGci"));
        assert!(!out.contains(openai));
        assert!(!out.contains(github));
        assert!(!out.contains(classic));
        assert_eq!(out.matches("[REDACTED]").count(), 4);
    }

    #[test]
    fn sanitize_issue_title_scrubs_plaintext_tokens() {
        let out = sanitize_issue_title("Bug with sk-ant-abcdefghijklmnopqrstuvwxyz123456");
        assert!(!out.contains("sk-ant"));
        assert!(out.contains("[REDACTED]"));
    }

    #[test]
    fn normalize_draft_applies_default_labels() {
        let cfg = IssueReportingConfig::default();
        let draft =
            normalize_draft(&cfg, IssueKind::Bug, "Broken thing", "?token=abc", None).unwrap();
        assert_eq!(draft.labels, vec!["bug"]);
        assert!(draft.body.contains("[REDACTED]"));
    }

    #[test]
    fn rejects_bad_repo_parts() {
        let cfg = IssueReportingConfig {
            owner: "bad/owner".into(),
            ..Default::default()
        };
        assert!(validate_repo_config(&cfg).is_err());
    }

    #[test]
    fn parses_issue_number_from_url() {
        assert_eq!(
            parse_issue_number_from_url("https://github.com/shiwenwen/hope-agent/issues/123"),
            Some(123)
        );
    }

    #[test]
    fn gh_repo_arg_uses_owner_repo_for_default_host() {
        let cfg = IssueReportingConfig::default();
        assert_eq!(gh_repo_arg(&cfg).unwrap(), "shiwenwen/hope-agent");
    }
}
