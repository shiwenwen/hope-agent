use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::globals::AppState;
use crate::recap::types::{GenerateMode, RecapFilters, RecapProgress};
use crate::recap::{generate_report, RecapContext};
use crate::slash_commands::types::{CommandAction, CommandResult};

/// Handle `/recap` inside a chat session.
///
/// - `/recap --full` → returns `OpenDashboardTab { tab: "recap" }` so the
///   frontend navigates to the Dashboard Recap tab.
/// - `/recap [--range=7d|--range=30d|--agent=<id>]` → spawns report
///   generation in the background and returns a `RecapCard { report_id }`
///   placeholder. The frontend subscribes to WS `recap_progress` events,
///   keyed by `report_id`, and renders a streaming card.
pub async fn handle_recap(
    state: &Arc<AppState>,
    _session_id: Option<&str>,
    args: &str,
) -> Result<CommandResult, String> {
    let args = args.trim();
    if args.contains("--full") {
        return Ok(CommandResult {
            content: "Opening Dashboard Recap tab…".into(),
            action: Some(CommandAction::OpenDashboardTab {
                tab: "recap".into(),
            }),
        });
    }

    let (mode, mode_desc) = parse_mode_from_args(args);
    let event_bus = crate::get_event_bus().cloned();

    let cancel = CancellationToken::new();
    let ctx = RecapContext::from_app_state(state, cancel)
        .await
        .map_err(|e| format!("recap init failed: {}", e))?;

    // Reserve a report_id up front so the frontend subscribes to the exact
    // same id the pipeline will stamp on its progress events.
    let report_id = uuid::Uuid::new_v4().to_string();
    let card_id = report_id.clone();
    let task_report_id = report_id.clone();

    tokio::spawn(async move {
        run_background(ctx, mode, task_report_id, event_bus).await;
    });

    Ok(CommandResult {
        content: format!(
            "Generating recap ({}). Stay here — a live summary will appear when ready.",
            mode_desc
        ),
        action: Some(CommandAction::RecapCard { report_id: card_id }),
    })
}

fn parse_mode_from_args(args: &str) -> (GenerateMode, String) {
    let mut days: Option<u32> = None;
    let mut agent: Option<String> = None;
    for tok in args.split_whitespace() {
        if let Some(v) = tok.strip_prefix("--range=") {
            if let Some(n) = v.strip_suffix('d').and_then(|s| s.parse::<u32>().ok()) {
                days = Some(n);
            }
        } else if let Some(v) = tok.strip_prefix("--agent=") {
            agent = Some(v.to_string());
        }
    }
    match days {
        Some(n) => {
            let end = chrono::Utc::now();
            let start = end - chrono::Duration::days(n as i64);
            let filters = RecapFilters {
                start_date: Some(start.format("%Y-%m-%d").to_string()),
                end_date: Some(end.format("%Y-%m-%d").to_string()),
                agent_id: agent,
                provider_id: None,
                model_id: None,
            };
            (
                GenerateMode::Full { filters },
                format!("last {} days", n),
            )
        }
        None => (GenerateMode::Incremental, "since last report".into()),
    }
}

async fn run_background(
    ctx: RecapContext,
    mode: GenerateMode,
    report_id: String,
    event_bus: Option<Arc<dyn crate::event_bus::EventBus>>,
) {
    let bus_for_emit = event_bus.clone();
    let id_for_emit = report_id.clone();
    let emit = move |progress: RecapProgress| {
        if let Some(bus) = bus_for_emit.as_ref() {
            bus.emit(
                "recap_progress",
                serde_json::json!({
                    "reportId": id_for_emit,
                    "progress": progress,
                }),
            );
        }
    };

    if let Err(e) = generate_report(&ctx, mode, report_id.clone(), emit).await {
        app_warn!("recap", "slash", "recap generate failed: {}", e);
        if let Some(bus) = event_bus.as_ref() {
            bus.emit(
                "recap_progress",
                serde_json::json!({
                    "reportId": report_id,
                    "progress": RecapProgress::Failed {
                        report_id: report_id.clone(),
                        message: e.to_string(),
                    },
                }),
            );
        }
    }
}
