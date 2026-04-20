//! Terminal-side first-run onboarding wizard + launch banner.
//!
//! Wired from `src-tauri/src/main.rs::run_server` (when stdin is a TTY)
//! and from the dedicated `hope-agent server setup` subcommand. The
//! wizard submodules own their prompting / persistence logic; the
//! banner and notice helpers live here so the Tauri command layer
//! (`commands::onboarding::list_local_ips`) can reach them too.

pub mod banner;
pub mod prompt;
pub mod steps;
pub mod wizard;

pub use banner::{print_launch_banner, print_unconfigured_notice};
pub use wizard::run as run_wizard;
