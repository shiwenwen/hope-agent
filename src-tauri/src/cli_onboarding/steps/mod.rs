//! CLI onboarding step prompters.
//!
//! One module per wizard step; each exposes a single `run(step, total)`
//! function that handles both user prompting and the corresponding
//! persistence call into `ha_core::onboarding::apply`.

pub mod channels;
pub mod import_openclaw;
pub mod language;
pub mod mode;
pub mod personality;
pub mod profile;
pub mod provider;
pub mod safety;
pub mod server;
pub mod skills;
pub mod summary;
