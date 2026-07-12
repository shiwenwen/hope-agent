use crate::commands::CmdError;
use ha_core::context_retrieval::{
    context_retrieval_for_session, ContextRetrievalInput, ContextRetrievalSnapshot,
};
use ha_core::session::SessionIdeContext;

#[tauri::command]
pub async fn get_context_retrieval(
    session_id: String,
    query: Option<String>,
    limit: Option<usize>,
    ide_context: Option<SessionIdeContext>,
    domain: Option<String>,
    template_id: Option<String>,
    template_version: Option<String>,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<ContextRetrievalSnapshot, CmdError> {
    context_retrieval_for_session(
        app_state.session_db.clone(),
        session_id,
        ContextRetrievalInput {
            query,
            limit,
            ide_context,
            domain,
            template_id,
            template_version,
        },
    )
    .await
    .map_err(Into::into)
}
