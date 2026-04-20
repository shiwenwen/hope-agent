//! Step 8 — IM channel reminder.
//!
//! The CLI wizard intentionally does NOT walk through 12 different
//! per-channel credential forms — that's a Web GUI experience. For
//! terminal users the pragmatic path is: list the available channels,
//! remind them they can configure any in Settings or via the web
//! interface, and move on. Anyone wanting richer CLI channel setup
//! should file a follow-up — the hooks in `ha-core::channel` already
//! exist, only the prompting layer is missing.

use anyhow::Result;

use crate::cli_onboarding::prompt::{print_saved, println_step};

const CHANNELS: &[&str] = &[
    "Telegram",
    "Discord",
    "Slack",
    "Feishu",
    "Google Chat",
    "LINE",
    "QQ Bot",
    "WhatsApp",
    "WeChat",
    "Signal",
    "IRC",
    "iMessage (macOS only)",
    "Email",
];

pub fn run(step: u32, total: u32) -> Result<()> {
    println_step(step, total, "IM channels (optional)");
    println!("  Hope Agent can relay chat through any of these IM integrations:");
    for (i, name) in CHANNELS.iter().enumerate() {
        println!("    {}. {}", i + 1, name);
    }
    println!();
    println!("  Full credential setup lives in the Web GUI → Settings → Channels.");
    println!("  Open the Web URL printed below after startup to continue.");
    print_saved("Channel step acknowledged");
    Ok(())
}
