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
        "elements" => {
            let request =
                serde_json::from_value::<crate::mac_control::MacControlElementsRequest>(
                    args.clone(),
                )?;
            Ok(serde_json::to_string_pretty(
                &crate::mac_control::elements(request).await,
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
        "windows" => {
            let request =
                serde_json::from_value::<crate::mac_control::MacControlWindowsRequest>(
                    args.clone(),
                )?;
            Ok(serde_json::to_string_pretty(
                &crate::mac_control::windows(request).await,
            )?)
        }
        "act" => {
            let request =
                serde_json::from_value::<crate::mac_control::MacControlActRequest>(args.clone())?;
            Ok(serde_json::to_string_pretty(
                &crate::mac_control::act(request).await,
            )?)
        }
        "menu" => {
            let request =
                serde_json::from_value::<crate::mac_control::MacControlMenuRequest>(args.clone())?;
            Ok(serde_json::to_string_pretty(
                &crate::mac_control::menu(request).await,
            )?)
        }
        "clipboard" => {
            let request = serde_json::from_value::<crate::mac_control::MacControlClipboardRequest>(
                args.clone(),
            )?;
            Ok(serde_json::to_string_pretty(
                &crate::mac_control::clipboard(request).await,
            )?)
        }
        "dialog" => {
            let request = serde_json::from_value::<crate::mac_control::MacControlDialogRequest>(
                args.clone(),
            )?;
            Ok(serde_json::to_string_pretty(
                &crate::mac_control::dialog(request).await,
            )?)
        }
        "visual" => {
            let request = serde_json::from_value::<crate::mac_control::MacControlVisualRequest>(
                args.clone(),
            )?;
            let response = crate::mac_control::visual(request).await;
            Ok(format_visual_response(&response)?)
        }
        other => bail!(
            "Unsupported mac_control action '{}'. Supported actions: 'status', 'permissions', 'snapshot', 'elements', 'wait', 'apps', 'windows', 'act', 'menu', 'clipboard', 'dialog', 'visual'.",
            other
        ),
    }
}

fn format_visual_response(
    response: &crate::mac_control::MacControlVisualResponse,
) -> Result<String> {
    let Some(result) = &response.result else {
        return Ok(serde_json::to_string_pretty(response)?);
    };
    if result.op != crate::mac_control::MacControlVisualOp::Observe {
        return Ok(serde_json::to_string_pretty(response)?);
    }
    let Some(screenshot) = &result.screenshot else {
        return Ok(serde_json::to_string_pretty(response)?);
    };
    let target = match screenshot.target {
        crate::mac_control::MacControlScreenshotTarget::Display => "display",
        crate::mac_control::MacControlScreenshotTarget::Window => "window",
    };
    let snapshot = result.snapshot.as_ref();
    let compact = serde_json::json!({
        "status": &response.status,
        "result": {
            "op": result.op,
            "snapshotId": &result.snapshot_id,
            "screenshot": screenshot,
            "frontmostApp": snapshot.and_then(|snapshot| snapshot.frontmost_app.as_ref()),
            "displays": snapshot.map(|snapshot| &snapshot.displays),
            "windows": snapshot.map(|snapshot| &snapshot.windows),
            "truncated": snapshot.map(|snapshot| snapshot.truncated),
            "warnings": snapshot.map(|snapshot| &snapshot.warnings).unwrap_or(&result.warnings),
        },
        "error": &response.error,
    });
    let compact_json = serde_json::to_string_pretty(&compact)?;
    let caption = format!(
        "mac_control visual.observe screenshot snapshotId={} target={} size={}x{} px. Use visual.point with coordinateSpace=\"image_pixels\" to convert a point from this image before clicking.",
        result.snapshot_id.as_deref().unwrap_or("unknown"),
        target,
        screenshot.width_px,
        screenshot.height_px
    );
    let marker = crate::tools::image_markers::build_image_file_marker(
        "image/jpeg",
        &screenshot.path,
        &format!("{caption}\n{compact_json}"),
    );
    Ok(marker)
}
