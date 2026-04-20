//! Step 3 — profile (name / timezone / experience / style).

use anyhow::Result;

use ha_core::onboarding::apply::{apply_profile, ProfileStepInput};

use crate::cli_onboarding::prompt::{print_saved, println_step, prompt_optional, prompt_select};

pub fn run(step: u32, total: u32) -> Result<()> {
    println_step(step, total, "Your profile (optional)");

    let system_tz = std::env::var("TZ").ok();
    let name = prompt_optional("Display name", None)?;
    let timezone = prompt_optional("Timezone", system_tz.as_deref())?;
    let experience_idx = prompt_select(
        "AI experience level:",
        &["Skip", "Beginner", "Intermediate", "Expert"],
        0,
    )?;
    let ai_experience = match experience_idx {
        1 => Some("beginner".to_string()),
        2 => Some("intermediate".to_string()),
        3 => Some("expert".to_string()),
        _ => None,
    };
    let style_idx = prompt_select(
        "Preferred response style:",
        &["Skip", "Concise", "Balanced", "Detailed"],
        0,
    )?;
    let response_style = match style_idx {
        1 => Some("concise".to_string()),
        2 => Some("balanced".to_string()),
        3 => Some("detailed".to_string()),
        _ => None,
    };

    apply_profile(ProfileStepInput {
        name,
        timezone,
        ai_experience,
        response_style,
    })?;
    print_saved("Profile saved");
    Ok(())
}
