//! Structured claim layer (next-gen Dreaming).
//!
//! Claims are the dual-track successor to flat `memories`: each is a
//! scoped, typed assertion (`subject predicate object`) with confidence,
//! salience, freshness policy, per-claim evidence, and links back to the
//! legacy `memories` rows it manages (design §2 / §3). They live in the same
//! `memory.db` (tables created in
//! [`crate::memory::sqlite::SqliteMemoryBackend::open`]).
//!
//! This module currently exposes the **read** surface only — the schema +
//! `claim_list` / `claim_get`. Claim extraction, legacy dual-write,
//! canonicalize / merge, and the prompt-injection path land in later PRs.

mod store;
mod types;

pub use store::{get_claim, init_claim_store, list_claims, ClaimListFilter};
pub use types::{ClaimDetail, ClaimLink, ClaimRecord, EvidenceRecord};
