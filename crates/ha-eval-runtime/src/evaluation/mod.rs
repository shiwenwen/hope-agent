//! Product control plane for the Evaluation Center.
//!
//! This module intentionally depends only on `ha-eval-spec`; the heavy Runner
//! remains the external `hope-agent-eval` Sidecar. Normal PR tests therefore
//! compile state, storage and protocol contracts without linking the scenario
//! pack or any real-model worker.

mod artifact_store;
mod evidence_bundle;
mod history;
mod local_bundle;
mod orchestrator;
mod provider_resolution;
mod query;
mod store;
mod types;

pub use artifact_store::{hex_digest as artifact_sha256, EvalArtifactStore, StoredEvalArtifact};
pub use evidence_bundle::{
    import_evidence_bundle, import_unverified_evidence_file, load_evidence_trust_registry_file,
    validate_evidence_trust_registry_file, verify_evidence_bundle, VerifiedEvidenceBundle,
};
pub use ha_eval_spec::app::{
    validate_app_control_envelope, AppControlCommand, AppControlEnvelope, AppControlEvent,
    AppControlHello, AppEvalSuiteCatalog, EvalAppPlan, EvalAppProfile, EvalAppRunRequest,
    APP_CONTROL_PROTOCOL_VERSION,
};
pub use ha_eval_spec::model::ModelCampaignTier;
pub use history::{
    coding_detail, domain_detail, CodingHistorySource, DomainHistorySource, EvalHistorySource,
};
pub use local_bundle::export_local_evidence_bundle;
pub use orchestrator::{EvalOrchestrator, EvalWorkerRuntime};
pub use provider_resolution::{
    list_model_options, resolve_local_launch, resolve_owner_provider_refs,
};
pub use query::EvalQueryService;
pub use store::EvalRepository;
pub use types::*;
