//! Read-only installation verification. Refresh observers without activating a
//! skill, injecting its body, changing settings, or dispatching a fork.

use anyhow::Result;
use serde_json::{json, Value};

use ha_core::tools::ToolExecContext;

use crate::skills::{self, SkillEntry};

pub(super) async fn execute(name: &str, ctx: &ToolExecContext) -> Result<String> {
    let name = name.to_owned();
    let workspace = ctx.session_working_dir.clone();
    let agent_id = ctx.agent_id.clone();
    let result = ha_core::blocking::run_blocking(move || {
        let cfg = ha_core::config::cached_config();
        // External installers cannot update the process-local version. Invalidate
        // the paths fast-path and notify catalog observers even on a lookup miss.
        skills::bump_skill_version();
        let entries = skills::load_all_skills_with_budget(
            &cfg.extra_skills_dirs,
            &cfg.skill_prompt_budget,
            workspace.as_deref().map(std::path::Path::new),
        );
        let entry = entries.iter().find(|entry| entry.name == name);
        let value = match entry {
            Some(entry) => {
                let requirements = skills::check_requirements_detail(
                    &entry.requires,
                    cfg.skill_env.get(&entry.name),
                );
                inspection(
                    entry,
                    !cfg.disabled_skills.contains(&entry.name),
                    skills::skill_env_check_enabled_for_agent(
                        agent_id.as_deref(),
                        cfg.skill_env_check,
                    ),
                    requirements,
                )
            }
            None => json!({"found": false, "name": name, "catalogRefreshed": true}),
        };
        app_info!(
            "skills",
            "inspect",
            "catalog refreshed; found={}",
            entry.is_some()
        );
        value
    })
    .await;
    Ok(serde_json::to_string(&result)?)
}

fn inspection(
    entry: &SkillEntry,
    enabled: bool,
    env_check: bool,
    requirements: skills::RequirementsDetail,
) -> Value {
    json!({
        "found": true,
        "catalogRefreshed": true,
        "name": entry.name,
        "source": entry.source,
        "baseDir": entry.base_dir,
        "filePath": entry.file_path,
        "enabled": enabled,
        "status": entry.status,
        "userInvocable": entry.user_invocable != Some(false),
        "modelInvocationAllowed": entry.disable_model_invocation != Some(true),
        "environmentCheckEnabled": env_check,
        "requirementsSatisfied": requirements.eligible,
        "hardBlocked": requirements.hard_blocked,
        "currentOs": requirements.current_os,
        "missingBins": requirements.missing_bins,
        "missingAnyBins": requirements.missing_any_bins,
        "missingEnv": requirements.missing_env,
        "missingConfig": requirements.missing_config,
        "supportedOs": requirements.supported_os,
        "paths": entry.paths,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn inspection_refreshes_and_resolves_project_skills_without_activating() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().canonicalize().unwrap();
        let name = "installer-inspection-fixture";
        let package = workspace.join(".hope-agent").join("skills").join(name);
        std::fs::create_dir_all(&package).unwrap();
        std::fs::write(
            package.join("SKILL.md"),
            format!(
                "---\nname: {name}\ndescription: inspection fixture\ncontext: fork\n\
                 user-invocable: false\ndisable-model-invocation: true\n---\n\
                 Inspection must not execute or return these instructions.\n"
            ),
        )
        .unwrap();
        let ctx = ToolExecContext {
            session_working_dir: Some(workspace.to_string_lossy().into_owned()),
            ..Default::default()
        };
        let version = skills::skill_cache_version();
        let args = json!({"name": name, "action": "inspect", "args": "Do not execute this"});
        let output = crate::tools::skill::tool_skill(&args, &ctx).await.unwrap();
        let result: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(result["source"], "project");
        assert_eq!(result["baseDir"], package.to_string_lossy().as_ref());
        assert_eq!(result["userInvocable"], false);
        assert_eq!(result["modelInvocationAllowed"], false);
        assert!(!output.contains("Inspection must not execute"));
        assert!(skills::skill_cache_version() > version);

        std::fs::remove_file(package.join("SKILL.md")).unwrap();
        let output = crate::tools::skill::tool_skill(&args, &ctx).await.unwrap();
        let result: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(result["found"], false);
        assert_eq!(result["catalogRefreshed"], true);
    }

    #[tokio::test]
    async fn inspection_unknown_action_does_not_fall_through_to_activation() {
        let result = crate::tools::skill::tool_skill(
            &json!({"name": "ha-skill-installer", "action": "install"}),
            &ToolExecContext::default(),
        )
        .await;
        assert!(result.unwrap_err().to_string().contains("'action' must be"));
    }

    #[test]
    fn inspection_preserves_readiness_constraints_without_returning_instructions() {
        let temp = tempfile::tempdir().unwrap();
        let package = temp.path().join("inspect-example");
        std::fs::create_dir(&package).unwrap();
        std::fs::write(
            package.join("SKILL.md"),
            "---\nname: inspect-example\ndescription: example\ncontext: fork\n\
             disable-model-invocation: true\npaths: [src/**]\n---\n\
             This body must never be returned by inspection.\n",
        )
        .unwrap();
        let entries =
            skills::load_all_skills_with_extra(&[temp.path().to_string_lossy().into_owned()], None);
        let entry = entries
            .iter()
            .find(|e| e.name == "inspect-example")
            .unwrap();
        let detail = skills::RequirementsDetail {
            eligible: false,
            missing_env: vec!["EXAMPLE_KEY".to_owned()],
            ..Default::default()
        };
        let value = inspection(entry, false, true, detail);
        assert_eq!(value["found"], true);
        assert_eq!(value["enabled"], false);
        assert_eq!(value["modelInvocationAllowed"], false);
        assert_eq!(value["requirementsSatisfied"], false);
        assert_eq!(value["missingEnv"], json!(["EXAMPLE_KEY"]));
        assert_eq!(value["paths"], json!(["src/**"]));
        assert_eq!(value["baseDir"], package.to_string_lossy().as_ref());
        assert!(!value.to_string().contains("This body"));
    }
}
