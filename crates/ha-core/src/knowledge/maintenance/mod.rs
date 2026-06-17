//! Knowledge Layer-2 autonomous maintenance (WS6).
//!
//! A background pipeline that periodically scans each knowledge base and queues
//! **maintenance proposals** (auto-link, orphan rescue, frontmatter fill, dedup
//! merge, knowledge gap, auto-tag, MOC upkeep, memory→note) into a draft review
//! queue. Nothing touches a user's notes until the owner approves a proposal in
//! the GUI (or, opt-in, `auto_approve` applies them inline).
//!
//! Mirrors the memory `dreaming` pipeline: Primary-instance gated, an
//! `AtomicBool` serial lock, idle + cron triggers, config-driven. The proposal
//! queue is the truth source in `sessions.db` (via [`KnowledgeRegistry`]); the
//! applier ([`apply`]) writes through the owner plane (`service`), bypassing the
//! agent-plane `effective_kb_access` because the owner explicitly approved.

pub mod apply;
pub mod config;
pub mod generators;
pub mod scheduler;
pub mod types;

pub use config::MaintenanceConfig;
pub use scheduler::{
    check_idle_trigger, maintenance_running, manual_run, spawn_maintenance_cron_loop,
    MaintenanceTrigger,
};
pub use types::{
    MaintenanceProposal, MaintenanceReport, MaintenanceStatus, NewProposal, ProposalAction,
    ProposalKind, ProposalStatus,
};

use anyhow::{anyhow, Result};

/// The owner-plane registry handle (truth source for the proposal queue).
fn registry() -> Result<&'static std::sync::Arc<super::KnowledgeRegistry>> {
    crate::get_knowledge_db().ok_or_else(|| anyhow!("knowledge db not initialized"))
}

/// List queued proposals for a KB (owner plane). `status=None` = all.
pub fn list_proposals(
    kb_id: &str,
    status: Option<ProposalStatus>,
) -> Result<Vec<MaintenanceProposal>> {
    registry()?.list_proposals(kb_id, status)
}

/// Pending (draft) proposal count for a KB (badge).
pub fn pending_count(kb_id: &str) -> Result<usize> {
    registry()?.count_pending_proposals(kb_id)
}

/// Approve a proposal: apply it through the owner plane, then mark Applied /
/// Failed. Returns the post-apply proposal row.
pub async fn approve_proposal(id: i64) -> Result<MaintenanceProposal> {
    let reg = registry()?;
    let proposal = reg
        .get_proposal(id)?
        .ok_or_else(|| anyhow!("proposal {id} not found"))?;
    if proposal.status != ProposalStatus::Draft {
        anyhow::bail!(
            "proposal {id} is not pending (status: {})",
            proposal.status.as_str()
        );
    }
    match apply::apply_proposal(&proposal).await {
        Ok(()) => {
            reg.set_proposal_status(id, ProposalStatus::Applied, None)?;
        }
        Err(e) => {
            reg.set_proposal_status(id, ProposalStatus::Failed, Some(&e.to_string()))?;
            return Err(e);
        }
    }
    reg.get_proposal(id)?
        .ok_or_else(|| anyhow!("proposal {id} vanished after apply"))
}

/// Reject a proposal (owner declined). Only a pending draft can be rejected, so a
/// stale double-click can't flip an already-applied proposal to rejected.
pub fn reject_proposal(id: i64) -> Result<()> {
    let reg = registry()?;
    let p = reg
        .get_proposal(id)?
        .ok_or_else(|| anyhow!("proposal {id} not found"))?;
    if p.status != ProposalStatus::Draft {
        anyhow::bail!(
            "proposal {id} is not pending (status: {})",
            p.status.as_str()
        );
    }
    reg.set_proposal_status(id, ProposalStatus::Rejected, None)
}

/// Reject all pending proposals for a KB (owner "clear queue"). Returns count.
pub fn reject_all(kb_id: &str) -> Result<usize> {
    scheduler::reject_all(kb_id)
}

/// Live status (running flag + last cycle report) for the GUI.
pub fn status() -> MaintenanceStatus {
    MaintenanceStatus {
        running: scheduler::maintenance_running(),
        last_report: scheduler::last_report(),
    }
}
