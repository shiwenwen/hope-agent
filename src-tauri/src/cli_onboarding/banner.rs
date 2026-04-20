//! Thin re-exports over [`ha_server::banner`] so the Tauri command layer
//! and (in PR 3) the interactive CLI wizard share one implementation with
//! the HTTP server itself. Keeping the module name stable lets PR 3 add
//! sibling prompt/step modules without a reshuffle.

pub use ha_server::banner::{
    display_host_urls, local_ipv4_addresses, print_launch_banner, print_unconfigured_notice,
};
