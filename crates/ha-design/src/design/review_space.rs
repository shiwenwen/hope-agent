//! 固定版本评审空间：scope 受限 bearer、过期/撤销、评论锚与本地审计。

use anyhow::{Context, Result};
use ha_core::platform::write_atomic;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const STORE_LOCK_TIMEOUT: Duration = Duration::from_secs(5);
const STORE_LOCK_POLL: Duration = Duration::from_millis(10);
const REVIEW_TOKEN_PREFIX: &str = "har1";
const REVIEW_TOKEN_SECRET_HEX_LEN: usize = 64;
const MAX_COMMENTS_PER_GRANT: usize = 500;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewRole {
    Viewer,
    Commenter,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewGrant {
    pub id: String,
    pub artifact_id: String,
    pub version_number: i64,
    pub role: ReviewRole,
    pub token_hash: String,
    pub expires_at: String,
    pub created_at: String,
    #[serde(default)]
    pub revoked_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewComment {
    pub id: String,
    pub grant_id: String,
    pub artifact_id: String,
    pub version_number: i64,
    #[serde(default)]
    pub oid: Option<i64>,
    pub rel_x: f64,
    pub rel_y: f64,
    pub body: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewAuditEvent {
    pub at: String,
    pub action: String,
    pub grant_id: String,
    #[serde(default)]
    pub comment_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReviewStore {
    version: u32,
    grants: Vec<ReviewGrant>,
    comments: Vec<ReviewComment>,
    audit: Vec<ReviewAuditEvent>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateReviewInput {
    pub artifact_id: String,
    pub version_number: i64,
    pub role: ReviewRole,
    pub expires_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatedReviewGrant {
    pub grant: ReviewGrant,
    /// 仅创建回执返回一次；磁盘只保存哈希。
    pub token: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddReviewCommentInput {
    #[serde(default)]
    pub oid: Option<i64>,
    pub rel_x: f64,
    pub rel_y: f64,
    pub body: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewSnapshot {
    pub artifact_id: String,
    pub version_number: i64,
    pub role: ReviewRole,
    pub html: String,
    pub comments: Vec<ReviewComment>,
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn token_hash(token: &str) -> String {
    blake3::hash(token.as_bytes()).to_hex().to_string()
}

fn create_review_token(artifact_id: &str) -> String {
    format!(
        "{REVIEW_TOKEN_PREFIX}.{artifact_id}.{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

fn review_token_artifact_id(token: &str) -> Option<&str> {
    let mut parts = token.split('.');
    if parts.next()? != REVIEW_TOKEN_PREFIX {
        return None;
    }
    let artifact_id = parts.next()?;
    let secret = parts.next()?;
    if parts.next().is_some()
        || uuid::Uuid::parse_str(artifact_id).is_err()
        || secret.len() != REVIEW_TOKEN_SECRET_HEX_LEN
        || !secret.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return None;
    }
    Some(artifact_id)
}

fn store_path(project_id: &str, artifact_id: &str) -> Result<PathBuf> {
    Ok(
        ha_core::paths::design_artifact_dir(project_id, artifact_id)?
            .join("review")
            .join("store.json"),
    )
}

fn load(path: &Path) -> Result<ReviewStore> {
    match std::fs::read(path) {
        Ok(bytes) => {
            let store: ReviewStore = serde_json::from_slice(&bytes)?;
            if store.version != 1 {
                anyhow::bail!("unsupported review store version");
            }
            Ok(store)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(ReviewStore {
            version: 1,
            ..Default::default()
        }),
        Err(e) => Err(e).context("read review store"),
    }
}

fn save(path: &Path, store: &ReviewStore) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    write_atomic(path, &serde_json::to_vec_pretty(store)?)?;
    Ok(())
}

fn acquire_store_lock(path: &Path) -> Result<std::fs::File> {
    let parent = path
        .parent()
        .context("review store path has no parent directory")?;
    std::fs::create_dir_all(parent)?;
    // The lock file is stable and separate from store.json, which is replaced
    // atomically. Locking store.json itself would lock the old inode on Unix and
    // stop protecting the newly renamed file.
    let lock_path = parent.join("store.lock");
    let started = Instant::now();
    loop {
        match ha_core::platform::try_acquire_exclusive_lock(&lock_path)? {
            Some(file) => return Ok(file),
            None if started.elapsed() < STORE_LOCK_TIMEOUT => std::thread::sleep(STORE_LOCK_POLL),
            None => anyhow::bail!("timed out waiting for the review store lock"),
        }
    }
}

pub(super) fn acquire_artifact_review_lock(
    project_id: &str,
    artifact_id: &str,
) -> Result<std::fs::File> {
    acquire_store_lock(&store_path(project_id, artifact_id)?)
}

fn acquire_live_artifact_lifecycle(
    artifact_id: &str,
    not_found_message: &str,
) -> Result<(super::DesignArtifact, std::fs::File)> {
    let candidate = super::service::get_artifact(artifact_id)?
        .ok_or_else(|| anyhow::anyhow!("{not_found_message}"))?;
    let guard =
        super::service::acquire_artifact_lifecycle_lock(&candidate.project_id, &candidate.id)?;
    let artifact = super::service::get_artifact(artifact_id)?
        .ok_or_else(|| anyhow::anyhow!("{not_found_message}"))?;
    if artifact.project_id != candidate.project_id {
        anyhow::bail!("{not_found_message}");
    }
    Ok((artifact, guard))
}

pub fn create(input: CreateReviewInput) -> Result<CreatedReviewGrant> {
    let (candidate, _lifecycle_guard) = acquire_live_artifact_lifecycle(
        &input.artifact_id,
        &format!("artifact not found: {}", input.artifact_id),
    )?;
    let artifact_guard = super::service::artifact_lock(&input.artifact_id);
    let _artifact_guard = artifact_guard.lock().unwrap_or_else(|e| e.into_inner());
    // The process-local mutex may have waited behind a version update. Re-read
    // while still holding the cross-process lifecycle guard before validating
    // and persisting a token for the exact snapshot.
    let artifact = super::service::get_artifact(&input.artifact_id)?
        .with_context(|| format!("artifact not found: {}", input.artifact_id))?;
    if artifact.project_id != candidate.project_id {
        anyhow::bail!("artifact identity changed while waiting for lifecycle lock");
    }
    if input.version_number <= 0 || input.version_number > artifact.current_version {
        anyhow::bail!("invalid artifact version");
    }
    let path = store_path(&artifact.project_id, &artifact.id)?;
    // This stable OS lock is shared with version cleanup in every process.
    // Holding it from exact-snapshot validation through grant persistence
    // prevents another Desktop/HTTP process from pruning between those steps.
    let _guard = acquire_store_lock(&path)?;
    // Retention can leave gaps below `current_version`; a fixed-version grant
    // is valid only when its exact immutable snapshot still exists.
    super::service::get_artifact_version_html(&artifact.id, input.version_number)?;
    let expires =
        chrono::DateTime::parse_from_rfc3339(&input.expires_at)?.with_timezone(&chrono::Utc);
    let now_dt = chrono::Utc::now();
    if expires <= now_dt || expires > now_dt + chrono::Duration::days(90) {
        anyhow::bail!("review expiry must be within the next 90 days");
    }
    let token = create_review_token(&artifact.id);
    let grant = ReviewGrant {
        id: uuid::Uuid::new_v4().to_string(),
        artifact_id: artifact.id.clone(),
        version_number: input.version_number,
        role: input.role,
        token_hash: token_hash(&token),
        expires_at: expires.to_rfc3339(),
        created_at: now_dt.to_rfc3339(),
        revoked_at: None,
    };
    let mut store = load(&path)?;
    store.grants.push(grant.clone());
    store.audit.push(ReviewAuditEvent {
        at: now(),
        action: "grant_created".into(),
        grant_id: grant.id.clone(),
        comment_id: None,
    });
    save(&path, &store)?;
    Ok(CreatedReviewGrant { grant, token })
}

pub fn list(artifact_id: &str) -> Result<Vec<ReviewGrant>> {
    let artifact = super::service::get_artifact(artifact_id)?
        .with_context(|| format!("artifact not found: {artifact_id}"))?;
    Ok(load(&store_path(&artifact.project_id, artifact_id)?)?.grants)
}

pub fn revoke(artifact_id: &str, grant_id: &str) -> Result<bool> {
    let (artifact, _lifecycle_guard) = acquire_live_artifact_lifecycle(
        artifact_id,
        &format!("artifact not found: {artifact_id}"),
    )?;
    let path = store_path(&artifact.project_id, artifact_id)?;
    let _guard = acquire_store_lock(&path)?;
    let mut store = load(&path)?;
    let Some(grant) = store
        .grants
        .iter_mut()
        .find(|g| g.id == grant_id && g.revoked_at.is_none())
    else {
        return Ok(false);
    };
    grant.revoked_at = Some(now());
    store.audit.push(ReviewAuditEvent {
        at: now(),
        action: "grant_revoked".into(),
        grant_id: grant_id.to_string(),
        comment_id: None,
    });
    save(&path, &store)?;
    Ok(true)
}

fn authorize(token: &str) -> Result<(PathBuf, ReviewStore, ReviewGrant)> {
    let artifact_id =
        review_token_artifact_id(token).ok_or_else(|| anyhow::anyhow!("review grant not found"))?;
    let artifact = super::service::get_artifact(artifact_id)?
        .ok_or_else(|| anyhow::anyhow!("review grant not found"))?;
    let hash = token_hash(token);
    let path = store_path(&artifact.project_id, &artifact.id)?;
    let store = load(&path)?;
    if let Some(grant) = store.grants.iter().find(|grant| {
        grant.artifact_id == artifact.id
            && grant.token_hash == hash
            && grant.revoked_at.is_none()
            && chrono::DateTime::parse_from_rfc3339(&grant.expires_at)
                .is_ok_and(|expires| expires.with_timezone(&chrono::Utc) > chrono::Utc::now())
    }) {
        return Ok((path, store.clone(), grant.clone()));
    }
    anyhow::bail!("review grant not found")
}

/// Caller must hold [`acquire_artifact_review_lock`] until DB and disk pruning
/// are both complete.
pub(super) fn active_version_numbers_under_lock(
    project_id: &str,
    artifact_id: &str,
) -> Result<std::collections::HashSet<i64>> {
    let now = chrono::Utc::now();
    Ok(load(&store_path(project_id, artifact_id)?)?
        .grants
        .into_iter()
        .filter(|grant| {
            grant.artifact_id == artifact_id
                && grant.revoked_at.is_none()
                && chrono::DateTime::parse_from_rfc3339(&grant.expires_at)
                    .is_ok_and(|expires| expires.with_timezone(&chrono::Utc) > now)
        })
        .map(|grant| grant.version_number)
        .collect())
}

pub fn snapshot(token: &str) -> Result<ReviewSnapshot> {
    let artifact_id =
        review_token_artifact_id(token).ok_or_else(|| anyhow::anyhow!("review grant not found"))?;
    let (candidate, _lifecycle_guard) =
        acquire_live_artifact_lifecycle(artifact_id, "review grant not found")?;
    let artifact_guard = super::service::artifact_lock(artifact_id);
    let _artifact_guard = artifact_guard.lock().unwrap_or_else(|e| e.into_inner());
    let artifact = super::service::get_artifact(artifact_id)?
        .ok_or_else(|| anyhow::anyhow!("review grant not found"))?;
    if artifact.project_id != candidate.project_id {
        anyhow::bail!("review grant not found");
    }
    let _review_guard = acquire_artifact_review_lock(&artifact.project_id, &artifact.id)?;
    let (_, store, grant) = authorize(token)?;
    let html = super::service::get_artifact_version_html(&grant.artifact_id, grant.version_number)?;
    Ok(ReviewSnapshot {
        artifact_id: grant.artifact_id.clone(),
        version_number: grant.version_number,
        role: grant.role,
        html,
        comments: store
            .comments
            .into_iter()
            .filter(|c| c.version_number == grant.version_number)
            .collect(),
    })
}

fn comment_quota_reached(store: &ReviewStore, grant_id: &str) -> bool {
    store
        .comments
        .iter()
        .filter(|comment| comment.grant_id == grant_id)
        .take(MAX_COMMENTS_PER_GRANT)
        .count()
        >= MAX_COMMENTS_PER_GRANT
}

pub fn add_comment(token: &str, input: AddReviewCommentInput) -> Result<ReviewComment> {
    let artifact_id =
        review_token_artifact_id(token).ok_or_else(|| anyhow::anyhow!("review grant not found"))?;
    let (artifact, _lifecycle_guard) =
        acquire_live_artifact_lifecycle(artifact_id, "review grant not found")?;
    let path = store_path(&artifact.project_id, &artifact.id)?;
    let _guard = acquire_store_lock(&path)?;
    let (_, _, grant) = authorize(token)?;
    let mut store = load(&path)?;
    let hash = token_hash(token);
    let live = store.grants.iter().any(|candidate| {
        candidate.id == grant.id
            && candidate.token_hash == hash
            && candidate.revoked_at.is_none()
            && chrono::DateTime::parse_from_rfc3339(&candidate.expires_at)
                .is_ok_and(|expires| expires.with_timezone(&chrono::Utc) > chrono::Utc::now())
    });
    if !live {
        anyhow::bail!("review grant not found");
    }
    if grant.role != ReviewRole::Commenter {
        anyhow::bail!("review grant is read-only");
    }
    let body = input.body.trim();
    if body.is_empty()
        || body.chars().count() > 2_000
        || !(0.0..=1.0).contains(&input.rel_x)
        || !(0.0..=1.0).contains(&input.rel_y)
    {
        anyhow::bail!("invalid review comment");
    }
    if comment_quota_reached(&store, &grant.id) {
        anyhow::bail!("review comment quota exceeded");
    }
    let comment = ReviewComment {
        id: uuid::Uuid::new_v4().to_string(),
        grant_id: grant.id.clone(),
        artifact_id: grant.artifact_id.clone(),
        version_number: grant.version_number,
        oid: input.oid,
        rel_x: input.rel_x,
        rel_y: input.rel_y,
        body: body.to_string(),
        created_at: now(),
    };
    store.comments.push(comment.clone());
    store.audit.push(ReviewAuditEvent {
        at: now(),
        action: "comment_added".into(),
        grant_id: grant.id,
        comment_id: Some(comment.id.clone()),
    });
    save(&path, &store)?;
    Ok(comment)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_tokens_are_directly_scoped_without_storing_the_secret() {
        let artifact_id = uuid::Uuid::new_v4().to_string();
        let token = create_review_token(&artifact_id);
        assert_eq!(review_token_artifact_id(&token), Some(artifact_id.as_str()));
        assert_ne!(token_hash(&token), token);

        assert_eq!(review_token_artifact_id(&"0".repeat(64)), None);
        assert_eq!(
            review_token_artifact_id(&format!("{REVIEW_TOKEN_PREFIX}.{artifact_id}.short")),
            None
        );
    }

    #[test]
    fn comment_quota_is_scoped_to_one_grant() {
        let comment = |id: usize, grant_id: &str| ReviewComment {
            id: format!("comment-{id}"),
            grant_id: grant_id.into(),
            artifact_id: "artifact".into(),
            version_number: 1,
            oid: None,
            rel_x: 0.5,
            rel_y: 0.5,
            body: "note".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
        };
        let mut store = ReviewStore {
            version: 1,
            ..Default::default()
        };
        store.comments.push(comment(0, "other-grant"));
        store
            .comments
            .extend((0..MAX_COMMENTS_PER_GRANT - 1).map(|id| comment(id + 1, "limited-grant")));
        assert!(!comment_quota_reached(&store, "limited-grant"));

        store
            .comments
            .push(comment(MAX_COMMENTS_PER_GRANT, "limited-grant"));
        assert!(comment_quota_reached(&store, "limited-grant"));
        assert!(!comment_quota_reached(&store, "other-grant"));
    }
}
