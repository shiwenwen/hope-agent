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
    let marker_screenshot = result.annotated_screenshot.as_ref().unwrap_or(screenshot);
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
            "annotatedScreenshot": &result.annotated_screenshot,
            "uiMap": &result.ui_map,
            "frontmostApp": snapshot.and_then(|snapshot| snapshot.frontmost_app.as_ref()),
            "displays": snapshot.map(|snapshot| &snapshot.displays),
            "windows": snapshot.map(|snapshot| &snapshot.windows),
            "truncated": snapshot.map(|snapshot| snapshot.truncated),
            "warnings": &result.warnings,
        },
        "error": &response.error,
    });
    let compact_json = serde_json::to_string_pretty(&compact)?;
    let annotation_hint = if result.annotated_screenshot.is_some() {
        " This image is annotated with AX element ids; prefer act.click target.elementId when a label is clear, or use visual.point for raw image pixels."
    } else {
        " Use visual.point with coordinateSpace=\"image_pixels\" to convert a point from this image before clicking."
    };
    let caption = format!(
        "mac_control visual.observe screenshot snapshotId={} target={} size={}x{} px.{}",
        result.snapshot_id.as_deref().unwrap_or("unknown"),
        target,
        marker_screenshot.width_px,
        marker_screenshot.height_px,
        annotation_hint
    );
    let marker = crate::tools::image_markers::build_image_file_marker(
        "image/jpeg",
        &marker_screenshot.path,
        &format!("{caption}\n{compact_json}"),
    );
    Ok(marker)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mac_control::{
        MacControlBounds, MacControlReadiness, MacControlRuntimeStats, MacControlScreenshotSummary,
        MacControlScreenshotTarget, MacControlStatus, MacControlVisualOp, MacControlVisualResponse,
        MacControlVisualResult, MacControlWindowSummary,
    };

    #[test]
    fn visual_observe_compact_response_preserves_result_warnings() {
        let response = MacControlVisualResponse {
            status: MacControlStatus {
                platform: "macos".to_string(),
                supported: true,
                desktop: true,
                bridge_registered: true,
                readiness: MacControlReadiness::Ready,
                core_ready: true,
                required_permissions: Vec::new(),
                optional_permissions: Vec::new(),
                missing_required: Vec::new(),
                optional_pending: Vec::new(),
                stats: MacControlRuntimeStats::default(),
                message: "ready".to_string(),
            },
            result: Some(MacControlVisualResult {
                op: MacControlVisualOp::Observe,
                snapshot_id: Some("macsnap_test".to_string()),
                snapshot: Some(crate::mac_control::MacControlSnapshot {
                    snapshot_id: "macsnap_test".to_string(),
                    created_at: "2026-05-23T00:00:00Z".to_string(),
                    frontmost_app: None,
                    displays: Vec::new(),
                    windows: vec![MacControlWindowSummary {
                        id: "win_1".to_string(),
                        app_pid: None,
                        role: None,
                        subrole: None,
                        title: None,
                        focused: true,
                        bounds_points: None,
                    }],
                    elements: Vec::new(),
                    screenshot: None,
                    truncated: false,
                    warnings: Vec::new(),
                }),
                screenshot: Some(MacControlScreenshotSummary {
                    media_id: "macsnap_test".to_string(),
                    path: "/tmp/macsnap_test.jpg".to_string(),
                    width_px: 10,
                    height_px: 10,
                    target: MacControlScreenshotTarget::Window,
                    display_id: None,
                    window_id: Some("win_1".to_string()),
                    window_title: None,
                    bounds_points: Some(MacControlBounds {
                        x: 0.0,
                        y: 0.0,
                        width: 10.0,
                        height: 10.0,
                    }),
                    scale: Some(1.0),
                }),
                annotated_screenshot: None,
                ui_map: Vec::new(),
                coordinate_space: None,
                image_point: None,
                screen_point: None,
                inside_frame: None,
                hit_elements: Vec::new(),
                nearest_elements: Vec::new(),
                text_blocks: Vec::new(),
                text_matches: Vec::new(),
                suggested_action: None,
                warnings: vec!["annotation failed".to_string()],
            }),
            error: None,
        };

        let formatted = format_visual_response(&response).expect("formatted response");
        let json_start = formatted.rfind("\n{").expect("compact json") + 1;
        let compact: serde_json::Value =
            serde_json::from_str(&formatted[json_start..]).expect("compact json");

        assert_eq!(
            compact["result"]["warnings"],
            serde_json::json!(["annotation failed"])
        );
    }
}
