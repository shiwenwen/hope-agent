use anyhow::{bail, Result};
use serde_json::Value;

pub(crate) async fn tool_mac_control(
    args: &Value,
    ctx: &ha_core::tools::ToolExecContext,
) -> Result<String> {
    let args = crate::sanitize_tool_args(args);
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("status");
    let started = std::time::Instant::now();
    let started_at = chrono::Utc::now().timestamp_millis();

    let result = match action {
        "status" => Ok(serde_json::to_string_pretty(
            &crate::status().await,
        )?),
        "permissions" => Ok(serde_json::to_string_pretty(
            &crate::permissions().await,
        )?),
        "diagnostics" => {
            let request =
                serde_json::from_value::<crate::MacControlDiagnosticsRequest>(
                    args.clone(),
                )?;
            Ok(serde_json::to_string_pretty(
                &crate::diagnostics(request).await,
            )?)
        }
        "snapshot" => {
            let request =
                serde_json::from_value::<crate::MacControlSnapshotRequest>(
                    args.clone(),
                )?;
            Ok(serde_json::to_string_pretty(
                &crate::snapshot(request).await,
            )?)
        }
        "elements" => {
            let request =
                serde_json::from_value::<crate::MacControlElementsRequest>(
                    args.clone(),
                )?;
            Ok(serde_json::to_string_pretty(
                &crate::elements(request).await,
            )?)
        }
        "wait" => {
            let request =
                serde_json::from_value::<crate::MacControlWaitRequest>(
                    args.clone(),
                )?;
            Ok(serde_json::to_string_pretty(
                &crate::wait(request).await,
            )?)
        }
        "apps" => {
            let request =
                serde_json::from_value::<crate::MacControlAppsRequest>(
                    args.clone(),
                )?;
            Ok(serde_json::to_string_pretty(
                &crate::apps(request).await,
            )?)
        }
        "dock" => {
            let request =
                serde_json::from_value::<crate::MacControlDockRequest>(
                    args.clone(),
                )?;
            Ok(serde_json::to_string_pretty(
                &crate::dock(request).await,
            )?)
        }
        "spaces" => {
            let request =
                serde_json::from_value::<crate::MacControlSpacesRequest>(
                    args.clone(),
                )?;
            Ok(serde_json::to_string_pretty(
                &crate::spaces(request).await,
            )?)
        }
        "windows" => {
            let request =
                serde_json::from_value::<crate::MacControlWindowsRequest>(
                    args.clone(),
                )?;
            Ok(serde_json::to_string_pretty(
                &crate::windows(request).await,
            )?)
        }
        "act" => {
            let request =
                serde_json::from_value::<crate::MacControlActRequest>(args.clone())?;
            Ok(serde_json::to_string_pretty(
                &crate::act(request).await,
            )?)
        }
        "menu" => {
            let request =
                serde_json::from_value::<crate::MacControlMenuRequest>(args.clone())?;
            Ok(serde_json::to_string_pretty(
                &crate::menu(request).await,
            )?)
        }
        "clipboard" => {
            let request = serde_json::from_value::<crate::MacControlClipboardRequest>(
                args.clone(),
            )?;
            Ok(serde_json::to_string_pretty(
                &crate::clipboard(request).await,
            )?)
        }
        "dialog" => {
            let request = serde_json::from_value::<crate::MacControlDialogRequest>(
                args.clone(),
            )?;
            Ok(serde_json::to_string_pretty(
                &crate::dialog(request).await,
            )?)
        }
        "visual" => {
            let request = serde_json::from_value::<crate::MacControlVisualRequest>(
                args.clone(),
            )?;
            let response = crate::visual(request).await;
            Ok(format_visual_response(&response)?)
        }
        other => bail!(
            "Unsupported mac_control action '{}'. Supported actions: 'status', 'permissions', 'diagnostics', 'snapshot', 'elements', 'wait', 'apps', 'dock', 'spaces', 'windows', 'act', 'menu', 'clipboard', 'dialog', 'visual'.",
            other
        ),
    };
    record_mac_control_action(&args, ctx, action, &result, started, started_at);
    result
}

/// Effective sub-op: explicit `op` from args, else the serde default the
/// request type would apply.
fn mac_effective_op<'a>(args: &'a Value, action: &str) -> Option<&'a str> {
    args.get("op").and_then(|v| v.as_str()).or(match action {
        "act" => Some("click"),
        "windows" | "menu" | "dock" | "spaces" => Some("list"),
        "apps" => Some("frontmost"),
        "dialog" => Some("inspect"),
        "clipboard" => Some("get"),
        _ => None,
    })
}

/// Timeline whitelist — only mutating steps; read-only queries (status /
/// snapshot / elements / lists / clipboard.get / act.dry_run) are skipped.
fn is_recordable_mac_action(action: &str, op: Option<&str>) -> bool {
    match action {
        "act" => !matches!(op, Some("dry_run")),
        "windows" => matches!(op, Some("focus" | "move" | "resize" | "minimize" | "close")),
        "menu" => matches!(op, Some("click" | "popover")),
        "dialog" => matches!(op, Some("click" | "input" | "file" | "accept" | "dismiss")),
        "dock" => matches!(op, Some("launch" | "hide" | "show" | "select_menu")),
        "apps" => matches!(op, Some("activate" | "launch" | "quit")),
        "spaces" => matches!(op, Some("switch" | "move_window")),
        "clipboard" => matches!(op, Some("set" | "clear")),
        _ => false,
    }
}

/// Redacted `(target, detail)` summary. Typed/pasted/clipboard text never
/// enters the payload — length only.
fn mac_action_summary(
    args: &Value,
    action: &str,
    op: Option<&str>,
) -> (Option<String>, Option<String>) {
    let target_obj = args.get("target");
    // Probe order mirrors MacControlTargetQuery's actual camelCase fields,
    // most descriptive first (element text match is the common targeting mode).
    let target = target_obj.and_then(|t| {
        ["text", "windowTitle", "appName", "elementId", "bundleId"]
            .iter()
            .find_map(|k| t.get(k).and_then(|v| v.as_str()))
            .map(str::to_string)
    });
    let text_summary = |key: &str| {
        args.get(key)
            .and_then(|v| v.as_str())
            .map(ha_core::tool_actions::redacted_text_summary)
    };
    let detail = match (action, op) {
        ("act", Some("type" | "paste")) => text_summary("text"),
        ("act", Some("set_value")) => text_summary("value").or_else(|| text_summary("text")),
        ("act", Some("hotkey" | "press")) => args
            .get("keys")
            .and_then(|v| v.as_array())
            .map(|keys| {
                format!(
                    "key={}",
                    keys.iter()
                        .filter_map(|k| k.as_str())
                        .collect::<Vec<_>>()
                        .join("+")
                )
            })
            .or_else(|| {
                args.get("key")
                    .and_then(|v| v.as_str())
                    .map(|k| format!("key={k}"))
            }),
        ("clipboard", Some("set")) => text_summary("text"),
        ("dialog", Some("input")) => text_summary("text"),
        ("dock" | "apps", _) => args
            .get("appName")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        _ => None,
    };
    (target, detail)
}

/// Choke-point recorder mirroring the browser tool: builds the redacted event,
/// pushes it into the ring buffer + EventBus, and fires the follow-up frame
/// capture (mutating success, plus `act` failure — screen may have changed).
fn record_mac_control_action(
    args: &Value,
    ctx: &ha_core::tools::ToolExecContext,
    action: &str,
    result: &Result<String>,
    started: std::time::Instant,
    started_at: i64,
) {
    let op = mac_effective_op(args, action);
    if !is_recordable_mac_action(action, op) {
        return;
    }
    // Responses embed their failure in an `error` field while the tool still
    // returns Ok(json) — probe it for the real outcome.
    let parsed = result
        .as_ref()
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(s).ok());
    let response_error = parsed
        .as_ref()
        .and_then(|v| v.get("error"))
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let ok = result.is_ok() && response_error.is_none();
    let app = parsed
        .as_ref()
        .and_then(|v| {
            v.pointer("/result/frontmostApp/name")
                .or_else(|| v.pointer("/frontmostApp/name"))
        })
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let error = result
        .as_ref()
        .err()
        .map(|e| e.to_string())
        .or(response_error)
        .map(|e| ha_core::tool_actions::clamp_error(&e));
    let emit_frame = ok || action == "act";
    let action_id = ha_core::tool_actions::new_action_id();
    let (target, detail) = mac_action_summary(args, action, op);
    ha_core::tool_actions::record_action(ha_core::tool_actions::ToolActionEvent {
        action_id: action_id.clone(),
        source: ha_core::tool_actions::ToolActionSource::MacControl,
        session_id: ctx.session_id.clone(),
        action: action.to_string(),
        op: op.map(str::to_string),
        target,
        detail,
        url: None,
        app,
        ok,
        error,
        duration_ms: started.elapsed().as_millis() as u64,
        started_at,
        tool_call_id: ctx.tool_call_id.clone(),
        has_frame: emit_frame,
    });
    if emit_frame {
        crate::capture_frame_for_action(action_id, ctx.session_id.clone());
    }
}

fn format_visual_response(response: &crate::MacControlVisualResponse) -> Result<String> {
    let Some(result) = &response.result else {
        return Ok(serde_json::to_string_pretty(response)?);
    };
    if result.op != crate::MacControlVisualOp::Observe {
        return Ok(serde_json::to_string_pretty(response)?);
    }
    let Some(screenshot) = &result.screenshot else {
        return Ok(serde_json::to_string_pretty(response)?);
    };
    let marker_screenshot = result.annotated_screenshot.as_ref().unwrap_or(screenshot);
    let target = match screenshot.target {
        crate::MacControlScreenshotTarget::Display => "display",
        crate::MacControlScreenshotTarget::Window => "window",
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
        " This image is annotated with AX element ids; prefer act.click target.elementId plus target.snapshotId when a label is clear, or use visual.point for raw image pixels."
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
    let marker = ha_core::tools::image_markers::build_image_file_marker(
        "image/jpeg",
        &marker_screenshot.path,
        &format!("{caption}\n{compact_json}"),
    );
    Ok(marker)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
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
                snapshot: Some(crate::MacControlSnapshot {
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
                suggested_actions: Vec::new(),
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

    /// Locks `mac_effective_op`'s hand-written default table to the actual
    /// serde `#[default]` variants — if a default moves, this fails instead
    /// of the recorder silently mislabeling op-omitted steps.
    #[test]
    fn mac_effective_op_defaults_match_serde_defaults() {
        fn serde_default<T: Default + serde::Serialize>() -> String {
            serde_json::to_value(T::default())
                .unwrap()
                .as_str()
                .unwrap()
                .to_string()
        }
        let empty = serde_json::json!({});
        let cases: &[(&str, String)] = &[
            ("act", serde_default::<crate::MacControlActOp>()),
            ("windows", serde_default::<crate::MacControlWindowsOp>()),
            ("menu", serde_default::<crate::MacControlMenuOp>()),
            ("dock", serde_default::<crate::MacControlDockOp>()),
            ("spaces", serde_default::<crate::MacControlSpacesOp>()),
            ("apps", serde_default::<crate::MacControlAppsOp>()),
            ("dialog", serde_default::<crate::MacControlDialogOp>()),
            ("clipboard", serde_default::<crate::MacControlClipboardOp>()),
        ];
        for (action, expected) in cases {
            assert_eq!(
                mac_effective_op(&empty, action),
                Some(expected.as_str()),
                "mac_effective_op default drifted for action '{action}'"
            );
        }
    }
}
