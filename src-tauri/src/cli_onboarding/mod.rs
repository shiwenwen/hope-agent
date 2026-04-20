//! Terminal-side first-run onboarding wizard + launch banner.
//!
//! PR 1 ships only the launch banner + non-TTY notice helpers. PR 3 fills
//! in the full interactive wizard (`run_wizard`, `run_reset_wizard`) and
//! the per-step prompters under `steps/`. Keeping the module scaffolded
//! now means the Tauri command layer (`commands::onboarding`) can depend
//! on `banner::local_ipv4_addresses()` without a follow-up refactor.

pub mod banner;

pub use banner::{print_launch_banner, print_unconfigured_notice};
