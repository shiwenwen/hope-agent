//! ACP server entry point — starts the stdio NDJSON server.

use std::sync::Arc;

use anyhow::Result;

use crate::acp::agent::AcpAgent;
use ha_core::session::SessionDB;

/// Start the ACP server, blocking on stdin/stdout.
pub fn start(
    session_db: Arc<SessionDB>,
    agent_id: String,
    verbose: bool,
    runtime: tokio::runtime::Handle,
) -> Result<()> {
    let mut agent = AcpAgent::new(session_db, agent_id, verbose, runtime);
    agent.run()
}
