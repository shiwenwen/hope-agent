use std::path::{Path, PathBuf};

use anyhow::Result;
use ha_core::personal_assistant_im::AssistantImError;

fn path(directory: &Path, reference: &str) -> Result<PathBuf> {
    uuid::Uuid::parse_str(reference).map_err(|_| AssistantImError::InvalidInput)?;
    Ok(directory.join(format!("personal-assistant-telegram-{reference}.token")))
}

fn valid_token(token: &str) -> bool {
    let Some((id, secret)) = token.split_once(':') else {
        return false;
    };
    !id.is_empty()
        && id.len() <= 20
        && id.bytes().all(|byte| byte.is_ascii_digit())
        && (20..=256).contains(&secret.len())
        && secret
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

/// A new immutable credential path is published before swapping the DB
/// reference. Readers never fall back to the old file after a rotation.
pub(super) fn publish(token: &str) -> Result<String> {
    publish_in(&ha_core::paths::credentials_dir()?, token)
}

fn publish_in(directory: &Path, token: &str) -> Result<String> {
    if !valid_token(token) {
        return Err(AssistantImError::InvalidInput.into());
    }
    std::fs::create_dir_all(directory)?;
    let reference = uuid::Uuid::new_v4().to_string();
    ha_core::platform::write_secure_file(&path(directory, &reference)?, token.as_bytes())?;
    Ok(reference)
}

pub(super) fn read(reference: &str) -> Result<String> {
    read_in(&ha_core::paths::credentials_dir()?, reference)
}

fn read_in(directory: &Path, reference: &str) -> Result<String> {
    let file = path(directory, reference)?;
    if std::fs::metadata(&file)?.len() > 512 {
        return Err(AssistantImError::Unavailable.into());
    }
    let token = std::fs::read_to_string(file)?;
    if !valid_token(&token) {
        return Err(AssistantImError::Unavailable.into());
    }
    Ok(token)
}

pub(super) fn remove(reference: &str) -> Result<()> {
    let file = path(&ha_core::paths::credentials_dir()?, reference)?;
    match std::fs::remove_file(file) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn immutable_secret_files_cannot_select_an_arbitrary_path() {
        let dir = tempfile::tempdir().unwrap();
        let first = "12345:abcdefghijklmnopqrstuvwxyz";
        let second = "12345:ABCDEFGHIJKLMNOPQRSTUVWXYZ";
        let a = publish_in(dir.path(), first).unwrap();
        let b = publish_in(dir.path(), second).unwrap();
        assert_ne!(a, b);
        assert_eq!(read_in(dir.path(), &a).unwrap(), first);
        assert_eq!(read_in(dir.path(), &b).unwrap(), second);
        assert!(read_in(dir.path(), "../config").is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(path(dir.path(), &a).unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }
}
