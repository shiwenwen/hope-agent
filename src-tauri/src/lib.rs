mod agent;

use agent::AssistantAgent;
use std::sync::Mutex;
use tauri::State;

struct AppState {
    agent: Mutex<Option<AssistantAgent>>,
}

#[tauri::command]
async fn initialize_agent(
    api_key: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let agent = AssistantAgent::new(&api_key);
    *state.agent.lock().unwrap() = Some(agent);
    Ok(())
}

#[tauri::command]
async fn chat(
    message: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let agent_lock = state.agent.lock().unwrap();
    match agent_lock.as_ref() {
        Some(agent) => agent.chat(&message).await.map_err(|e| e.to_string()),
        None => Err("Agent not initialized. Please provide an API key.".to_string()),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }
            Ok(())
        })
        .manage(AppState {
            agent: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![initialize_agent, chat])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
