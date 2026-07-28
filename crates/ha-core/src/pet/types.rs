use serde::{Deserialize, Deserializer, Serialize, Serializer};

// 类型已下沉 ha-config-schema（config.json wire 类型），原地再导出保持路径不变。
pub use ha_config_schema::pet::{PetConfig, PetRef, BUILTIN_DEFAULT_PET_REF};

#[cfg(debug_assertions)]
pub const BUILTIN_DEBUG_PET_REF: &str = "builtin:hope-debug";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PetSpriteVersion {
    V1,
    V2,
}

impl PetSpriteVersion {
    pub const fn number(self) -> u8 {
        match self {
            Self::V1 => 1,
            Self::V2 => 2,
        }
    }

    pub const fn row_count(self) -> u32 {
        match self {
            Self::V1 => 9,
            Self::V2 => 11,
        }
    }
}

impl TryFrom<u8> for PetSpriteVersion {
    type Error = String;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::V1),
            2 => Ok(Self::V2),
            _ => Err("spriteVersionNumber must be 1 or 2".to_string()),
        }
    }
}

impl Serialize for PetSpriteVersion {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u8(self.number())
    }
}

impl<'de> Deserialize<'de> for PetSpriteVersion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::try_from(u8::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PetManifest {
    pub id: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default = "default_sprite_version")]
    pub sprite_version_number: PetSpriteVersion,
    #[serde(default = "default_spritesheet_path")]
    pub spritesheet_path: String,
}

fn default_sprite_version() -> PetSpriteVersion {
    PetSpriteVersion::V1
}

fn default_spritesheet_path() -> String {
    "spritesheet.webp".to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PetSourceKind {
    Builtin,
    Codex,
    File,
    Zip,
    Url,
    Created,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HopePetMetadata {
    pub schema_version: u32,
    pub source_kind: PetSourceKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    pub package_hash: String,
    pub asset_hash: String,
    pub imported_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PetSummary {
    pub pet_ref: PetRef,
    pub manifest: PetManifest,
    pub asset_id: String,
    pub source_kind: PetSourceKind,
    pub package_hash: String,
    pub builtin: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PetLibrarySnapshot {
    pub revision: u64,
    pub selected_pet_ref: PetRef,
    pub selected_pet_available: bool,
    pub pets: Vec<PetSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PetValidationSeverity {
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PetValidationIssue {
    pub code: String,
    pub severity: PetValidationSeverity,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PetImportCandidate {
    pub candidate_id: String,
    pub display_name: String,
    pub source_kind: String,
    pub width: u32,
    pub height: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inferred_version: Option<PetSpriteVersion>,
    pub issues: Vec<PetValidationIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PetCandidatePage {
    pub candidates: Vec<PetImportCandidate>,
    pub total: u32,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum PetImportSource {
    Candidate {
        candidate_id: String,
    },
    LocalPath {
        path: String,
    },
    LocalPaths {
        paths: Vec<String>,
    },
    UploadedPath {
        upload_id: String,
    },
    UploadedFiles {
        upload_ids: Vec<String>,
    },
    /// A pasted Codex/Hope install link or a direct HTTPS spritesheet URL.
    /// The URL is fetched only into the bounded preview cache and never sent
    /// to the renderer as an image source.
    Link {
        link: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PetCreateRequest {
    pub display_name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub prompt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PetExportResult {
    pub file_name: String,
    pub mime: String,
    pub data_base64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PetImportPreviewRequest {
    pub source: PetImportSource,
    #[serde(default)]
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PetImportPreview {
    pub preview_token: String,
    pub manifest: PetManifest,
    pub width: u32,
    pub height: u32,
    pub issues: Vec<PetValidationIssue>,
    pub asset_hash: String,
    pub package_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duplicate_pet_ref: Option<PetRef>,
}

impl PetImportPreview {
    pub fn can_commit(&self) -> bool {
        !self
            .issues
            .iter()
            .any(|issue| issue.severity == PetValidationSeverity::Error)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PetImportCommitRequest {
    pub preview_token: String,
    #[serde(default)]
    pub enable_after_import: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PetImportCommitResult {
    pub pet: PetSummary,
    pub imported: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PetDeleteResult {
    pub restore_token: String,
    pub deleted_pet_ref: PetRef,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PetRestoreRequest {
    pub restore_token: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatUiSurface {
    MainChat,
    QuickChat,
    KnowledgeChat,
    DesignChat,
    PetChat,
}

impl ChatUiSurface {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MainChat => "main_chat",
            Self::QuickChat => "quick_chat",
            Self::KnowledgeChat => "knowledge_chat",
            Self::DesignChat => "design_chat",
            Self::PetChat => "pet_chat",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "main_chat" => Some(Self::MainChat),
            "quick_chat" => Some(Self::QuickChat),
            "knowledge_chat" => Some(Self::KnowledgeChat),
            "design_chat" => Some(Self::DesignChat),
            "pet_chat" => Some(Self::PetChat),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PetActivityStatus {
    NeedsInput,
    Blocked,
    Ready,
    Running,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PetActivityTitleKind {
    Session,
    Incognito,
    Untitled,
}

impl PetActivityStatus {
    pub const fn priority(self) -> u8 {
        match self {
            Self::NeedsInput => 0,
            Self::Blocked => 1,
            Self::Ready => 2,
            Self::Running => 3,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum PetNavigationTarget {
    Regular {
        session_id: String,
        project_id: Option<String>,
    },
    Knowledge {
        session_id: String,
        kb_id: String,
        anchor_note_path: Option<String>,
    },
    Design {
        session_id: String,
        project_id: String,
        artifact_id: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PetActivity {
    pub activity_id: String,
    pub status: PetActivityStatus,
    pub title: Option<String>,
    pub title_kind: PetActivityTitleKind,
    /// Redacted for incognito conversations.
    pub agent_id: Option<String>,
    pub updated_at: String,
    pub boundary: Option<i64>,
    /// Single-line, bounded preview of the terminal assistant response. Only
    /// populated for non-incognito ready activities; running/pending/error
    /// copy stays localised in the renderer.
    pub preview: Option<String>,
    pub target: PetNavigationTarget,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PetActivitySnapshot {
    pub revision: u64,
    pub generated_at: String,
    pub stale: bool,
    pub dominant: Option<PetActivityStatus>,
    pub activities: Vec<PetActivity>,
    pub total: u32,
    pub truncated: bool,
}

#[derive(Debug, Clone)]
pub struct InstalledSprite {
    pub path: Option<std::path::PathBuf>,
    pub mime: &'static str,
    pub etag: String,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{PetImportSource, PetNavigationTarget};

    #[test]
    fn import_sources_use_camel_case_variant_fields_on_the_wire() {
        let fixtures = [
            json!({ "kind": "candidate", "candidateId": "candidate-1" }),
            json!({ "kind": "localPath", "path": "/tmp/pet.json" }),
            json!({ "kind": "localPaths", "paths": ["/tmp/pet.json", "/tmp/pet.webp"] }),
            json!({ "kind": "uploadedPath", "uploadId": "upload-1" }),
            json!({ "kind": "uploadedFiles", "uploadIds": ["upload-1", "upload-2"] }),
            json!({ "kind": "link", "link": "https://example.com/pet.webp" }),
        ];

        for fixture in fixtures {
            let source: PetImportSource =
                serde_json::from_value(fixture.clone()).expect("deserialize import source");
            assert_eq!(
                serde_json::to_value(source).expect("serialize import source"),
                fixture
            );
        }
    }

    #[test]
    fn navigation_targets_use_camel_case_variant_fields_on_the_wire() {
        let fixtures = [
            json!({
                "kind": "regular",
                "sessionId": "session-1",
                "projectId": "project-1"
            }),
            json!({
                "kind": "knowledge",
                "sessionId": "session-2",
                "kbId": "kb-1",
                "anchorNotePath": "notes/example.md"
            }),
            json!({
                "kind": "design",
                "sessionId": "session-3",
                "projectId": "design-1",
                "artifactId": "artifact-1"
            }),
        ];

        for fixture in fixtures {
            let target: PetNavigationTarget =
                serde_json::from_value(fixture.clone()).expect("deserialize navigation target");
            assert_eq!(
                serde_json::to_value(target).expect("serialize navigation target"),
                fixture
            );
        }
    }
}
