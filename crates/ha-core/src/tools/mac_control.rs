use anyhow::{bail, Result};
use serde_json::Value;

pub(crate) async fn tool_mac_control(args: &Value) -> Result<String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("status");

    match action {
        "status" => Ok(serde_json::to_string_pretty(
            &crate::mac_control::status().await,
        )?),
        "permissions" => Ok(serde_json::to_string_pretty(
            &crate::mac_control::permissions().await,
        )?),
        "snapshot" => {
            let request =
                serde_json::from_value::<crate::mac_control::MacControlSnapshotRequest>(
                    args.clone(),
                )?;
            Ok(serde_json::to_string_pretty(
                &crate::mac_control::snapshot(request).await,
            )?)
        }
        "wait" => {
            let request =
                serde_json::from_value::<crate::mac_control::MacControlWaitRequest>(
                    args.clone(),
                )?;
            Ok(serde_json::to_string_pretty(
                &crate::mac_control::wait(request).await,
            )?)
        }
        "apps" => {
            let request =
                serde_json::from_value::<crate::mac_control::MacControlAppsRequest>(
                    args.clone(),
                )?;
            Ok(serde_json::to_string_pretty(
                &crate::mac_control::apps(request).await,
            )?)
        }
        other => bail!(
            "Unsupported mac_control action '{}'. Supported actions: 'status', 'permissions', 'snapshot', 'wait', 'apps'.",
            other
        ),
    }
}
