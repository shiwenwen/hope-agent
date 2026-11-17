use super::{emit_inbox_changed, AssistantProfile};
use crate::session::SessionDB;
use anyhow::{bail, Result};
use rusqlite::{params, OptionalExtension};

pub fn default_assistant_profile() -> AssistantProfile {
    AssistantProfile {
        name: "Hope".into(),
        avatar: None,
    }
}

fn validate_profile(mut profile: AssistantProfile) -> Result<AssistantProfile> {
    profile.name = profile.name.trim().to_owned();
    if profile.name.is_empty()
        || profile.name.chars().count() > 32
        || profile.name.chars().any(char::is_control)
    {
        bail!("Assistant name must contain 1–32 characters without control characters");
    }
    if let Some(avatar) = profile.avatar.as_deref() {
        if !matches!(avatar, "preset:sparkles" | "preset:bot" | "preset:heart") {
            // Never turn an owner-provided URL or host path into an asset route.
            // Uploaded pictures must be regular image files in the avatar store.
            let root = crate::paths::avatars_dir()?.canonicalize()?;
            let path = std::path::Path::new(avatar).canonicalize()?;
            let extension = path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            if path.parent() != Some(root.as_path())
                || !path.is_file()
                || !matches!(extension.as_str(), "png" | "jpg" | "jpeg" | "webp")
            {
                bail!("Assistant avatar must be an uploaded image or a supported preset");
            }
            profile.avatar = Some(path.to_string_lossy().into_owned());
        }
    }
    Ok(profile)
}

impl SessionDB {
    /// Owner UI only. Reading a default profile creates neither a home nor data.
    pub fn personal_assistant_profile(&self) -> Result<AssistantProfile> {
        let conn = self.read_conn()?;
        Ok(conn
            .query_row(
                "SELECT name,avatar FROM personal_assistant_profile WHERE singleton=1",
                [],
                |row| {
                    Ok(AssistantProfile {
                        name: row.get(0)?,
                        avatar: row.get(1)?,
                    })
                },
            )
            .optional()?
            .unwrap_or_else(default_assistant_profile))
    }

    /// Changes presentation only: no session, Agent, request or grant is edited.
    pub fn update_personal_assistant_profile(
        &self,
        profile: AssistantProfile,
    ) -> Result<AssistantProfile> {
        let profile = validate_profile(profile)?;
        {
            let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
            conn.execute(
                "INSERT INTO personal_assistant_profile(singleton,name,avatar) VALUES(1,?1,?2)
                 ON CONFLICT(singleton) DO UPDATE SET name=excluded.name,avatar=excluded.avatar",
                params![profile.name, profile.avatar],
            )?;
        }
        emit_inbox_changed();
        Ok(profile)
    }
}
