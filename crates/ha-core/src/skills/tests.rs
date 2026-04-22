#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use crate::skills::discovery::compact_path;
    use crate::skills::frontmatter::{
        parse_bool_value, parse_frontmatter, parse_install_specs, parse_requires, unquote,
        ParsedFrontmatter,
    };
    use crate::skills::prompt::build_skills_prompt;
    use crate::skills::requirements::{
        check_requirements, check_requirements_detail, is_masked_value, mask_value,
    };
    use crate::skills::slash::{check_all_skills_status, normalize_skill_command_name};
    use crate::skills::types::*;

    fn make_skill(name: &str, desc: &str) -> SkillEntry {
        SkillEntry {
            name: name.to_string(),
            aliases: Vec::new(),
            description: desc.to_string(),
            when_to_use: None,
            source: "managed".to_string(),
            file_path: format!("/tmp/{}/SKILL.md", name),
            base_dir: format!("/tmp/{}", name),
            requires: SkillRequires::default(),
            skill_key: None,
            user_invocable: None,
            disable_model_invocation: None,
            command_dispatch: None,
            command_tool: None,
            command_arg_mode: None,
            command_arg_placeholder: None,
            command_arg_options: None,
            command_prompt_template: None,
            install: vec![],
            allowed_tools: vec![],
            context_mode: None,
            agent: None,
            effort: None,
            paths: None,
            status: SkillStatus::Active,
            authored_by: None,
            rationale: None,
        }
    }

    fn make_skill_with_path(name: &str, desc: &str, path: &str) -> SkillEntry {
        SkillEntry {
            name: name.to_string(),
            aliases: Vec::new(),
            description: desc.to_string(),
            when_to_use: None,
            source: "managed".to_string(),
            file_path: path.to_string(),
            base_dir: format!("/tmp/{}", name),
            requires: SkillRequires::default(),
            skill_key: None,
            user_invocable: None,
            disable_model_invocation: None,
            command_dispatch: None,
            command_tool: None,
            command_arg_mode: None,
            command_arg_placeholder: None,
            command_arg_options: None,
            command_prompt_template: None,
            install: vec![],
            allowed_tools: vec![],
            context_mode: None,
            agent: None,
            effort: None,
            paths: None,
            status: SkillStatus::Active,
            authored_by: None,
            rationale: None,
        }
    }

    fn parse_bundled_skill_frontmatter(name: &str) -> ParsedFrontmatter {
        let skill_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("skills")
            .join(name)
            .join("SKILL.md");
        let content = std::fs::read_to_string(&skill_path)
            .unwrap_or_else(|e| panic!("failed to read {}: {}", skill_path.display(), e));
        parse_frontmatter(&content)
            .unwrap_or_else(|| panic!("failed to parse frontmatter for {}", skill_path.display()))
    }

    #[test]
    fn test_parse_frontmatter_basic() {
        let content = r#"---
name: github
description: "GitHub operations via gh CLI"
---

# GitHub Skill

Use the gh CLI.
"#;
        let parsed = parse_frontmatter(content).unwrap();
        assert_eq!(parsed.name, "github");
        assert_eq!(parsed.description, "GitHub operations via gh CLI");
        assert!(parsed.body.contains("# GitHub Skill"));
        assert!(parsed.skill_key.is_none());
        assert!(parsed.user_invocable.is_none());
    }

    #[test]
    fn test_parse_frontmatter_extended() {
        let content = r#"---
name: slack
description: "Slack messaging"
skillKey: slack-custom
user-invocable: true
disable-model-invocation: false
command-dispatch: tool
command-tool: slack_send
---

Body
"#;
        let parsed = parse_frontmatter(content).unwrap();
        assert_eq!(parsed.name, "slack");
        assert_eq!(parsed.skill_key.as_deref(), Some("slack-custom"));
        assert_eq!(parsed.user_invocable, Some(true));
        assert_eq!(parsed.disable_model_invocation, Some(false));
        assert_eq!(parsed.command_dispatch.as_deref(), Some("tool"));
        assert_eq!(parsed.command_tool.as_deref(), Some("slack_send"));
    }

    #[test]
    fn test_parse_frontmatter_unquoted() {
        let content = "---\nname: my-skill\ndescription: A simple skill\n---\nBody here";
        let parsed = parse_frontmatter(content).unwrap();
        assert_eq!(parsed.name, "my-skill");
        assert_eq!(parsed.description, "A simple skill");
    }

    #[test]
    fn test_parse_frontmatter_agent_and_effort() {
        let content = r#"---
name: heavy-skill
description: A deep-reasoning skill
context: fork
agent: "code-reviewer"
effort: high
---

Body
"#;
        let parsed = parse_frontmatter(content).unwrap();
        assert_eq!(parsed.context_mode.as_deref(), Some("fork"));
        assert_eq!(parsed.agent.as_deref(), Some("code-reviewer"));
        assert_eq!(parsed.effort.as_deref(), Some("high"));
    }

    #[test]
    fn test_parse_frontmatter_agent_effort_absent() {
        // Leaving agent:/effort: unset must remain None, not empty-string.
        let content = "---\nname: minimal\ndescription: no overrides\n---\nBody";
        let parsed = parse_frontmatter(content).unwrap();
        assert!(parsed.agent.is_none());
        assert!(parsed.effort.is_none());
    }

    #[test]
    fn test_parse_frontmatter_aliases() {
        let content = r#"---
name: review-pr
description: PR review
aliases: [pr-review, reviewpr]
---

Body
"#;
        let parsed = parse_frontmatter(content).unwrap();
        assert_eq!(parsed.aliases, vec!["pr-review", "reviewpr"]);
    }

    #[test]
    fn test_parse_frontmatter_aliases_absent() {
        let content = "---\nname: minimal\ndescription: no aliases\n---\nBody";
        let parsed = parse_frontmatter(content).unwrap();
        assert!(parsed.aliases.is_empty());
    }

    #[test]
    fn test_parse_frontmatter_when_to_use() {
        // All three spellings should populate the same slot.
        for key in ["whenToUse", "when-to-use", "when_to_use"] {
            let content = format!(
                "---\nname: s\ndescription: x\n{}: when user asks about Y\n---\nBody",
                key
            );
            let parsed = parse_frontmatter(&content).unwrap();
            assert_eq!(
                parsed.when_to_use.as_deref(),
                Some("when user asks about Y"),
                "key {} should parse",
                key
            );
        }
    }

    #[test]
    fn test_parse_frontmatter_argument_hint_alias() {
        // argumentHint / argument-hint / argument_hint are all aliases for
        // command-arg-placeholder — all should populate the same field.
        for key in ["argumentHint", "argument-hint", "argument_hint"] {
            let content = format!(
                "---\nname: s\ndescription: x\n{}: \"<query>\"\n---\nBody",
                key
            );
            let parsed = parse_frontmatter(&content).unwrap();
            assert_eq!(
                parsed.command_arg_placeholder.as_deref(),
                Some("<query>"),
                "key {} should map to command_arg_placeholder",
                key
            );
        }
    }

    #[test]
    fn test_parse_frontmatter_missing_name() {
        let content = "---\ndescription: No name\n---\nBody";
        assert!(parse_frontmatter(content).is_none());
    }

    #[test]
    fn test_parse_frontmatter_no_frontmatter() {
        let content = "Just regular markdown";
        assert!(parse_frontmatter(content).is_none());
    }

    #[test]
    fn test_bundled_core_skills_are_always_available() {
        for name in ["ha-settings", "ha-skill-creator", "ha-find-skills"] {
            let parsed = parse_bundled_skill_frontmatter(name);
            assert!(
                parsed.requires.always,
                "{} should declare always: true in its bundled SKILL.md",
                name
            );
        }
    }

    #[test]
    fn test_parse_requires_inline() {
        let yaml = "name: git\ndescription: d\nrequires:\n  bins: [git, gh]\n  env: [GITHUB_TOKEN]\n  os: [darwin, linux]\n";
        let req = parse_requires(yaml);
        assert_eq!(req.bins, vec!["git", "gh"]);
        assert_eq!(req.env, vec!["GITHUB_TOKEN"]);
        assert_eq!(req.os, vec!["darwin", "linux"]);
    }

    #[test]
    fn test_parse_requires_list_style() {
        let yaml = "name: git\ndescription: d\nrequires:\n  bins:\n    - git\n    - gh\n  env:\n    - GITHUB_TOKEN\n";
        let req = parse_requires(yaml);
        assert_eq!(req.bins, vec!["git", "gh"]);
        assert_eq!(req.env, vec!["GITHUB_TOKEN"]);
    }

    #[test]
    fn test_parse_requires_any_bins() {
        let yaml = "name: test\ndescription: d\nrequires:\n  anyBins: [rg, grep]\n  bins: [git]\n";
        let req = parse_requires(yaml);
        assert_eq!(req.bins, vec!["git"]);
        assert_eq!(req.any_bins, vec!["rg", "grep"]);
    }

    #[test]
    fn test_parse_requires_always() {
        let yaml = "name: test\ndescription: d\nalways: true\nrequires:\n  bins: [nonexistent_binary_xyz]\n";
        let req = parse_requires(yaml);
        assert!(req.always);
        assert_eq!(req.bins, vec!["nonexistent_binary_xyz"]);
    }

    #[test]
    fn test_parse_requires_primary_env() {
        let yaml =
            "name: test\ndescription: d\nprimaryEnv: MY_API_KEY\nrequires:\n  env: [MY_API_KEY]\n";
        let req = parse_requires(yaml);
        assert_eq!(req.primary_env.as_deref(), Some("MY_API_KEY"));
        assert_eq!(req.env, vec!["MY_API_KEY"]);
    }

    #[test]
    fn test_parse_requires_config() {
        let yaml = "name: test\ndescription: d\nrequires:\n  config: [webSearch.provider]\n";
        let req = parse_requires(yaml);
        assert_eq!(req.config, vec!["webSearch.provider"]);
    }

    #[test]
    fn test_parse_install_specs() {
        let yaml = r#"name: test
description: d
install:
  - kind: brew
    formula: gh
    bins: [gh]
    label: "Install GitHub CLI"
  - kind: node
    package: "@anthropic-ai/sdk"
"#;
        let specs = parse_install_specs(yaml);
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].kind, "brew");
        assert_eq!(specs[0].formula.as_deref(), Some("gh"));
        assert_eq!(specs[0].bins, vec!["gh"]);
        assert_eq!(specs[0].label.as_deref(), Some("Install GitHub CLI"));
        assert_eq!(specs[1].kind, "node");
        assert_eq!(specs[1].package.as_deref(), Some("@anthropic-ai/sdk"));
    }

    #[test]
    fn test_build_skills_prompt_empty() {
        assert_eq!(
            build_skills_prompt(
                &[],
                &[],
                false,
                &HashMap::new(),
                &SkillPromptBudget::default(),
                &[],
                &std::collections::HashSet::new(),
            ),
            ""
        );
    }

    #[test]
    fn test_build_skills_prompt_full_format() {
        let skills = vec![make_skill_with_path(
            "github",
            "GitHub ops",
            "/home/user/skills/github/SKILL.md",
        )];
        let prompt = build_skills_prompt(
            &skills,
            &[],
            false,
            &HashMap::new(),
            &SkillPromptBudget::default(),
            &[],
            &std::collections::HashSet::new(),
        );
        // Catalog entries no longer expose file paths — the `skill` tool looks
        // skills up by name instead of instructing the model to `read` SKILL.md.
        assert!(prompt.contains("- github: GitHub ops"));
        assert!(prompt.contains("skill` tool"));
        // The list line for this skill must be free of any path reference;
        // the instructional header still mentions SKILL.md as a don't-do-this.
        let list_line = prompt
            .lines()
            .find(|l| l.starts_with("- github"))
            .expect("list line present");
        assert!(!list_line.contains("SKILL.md"));
        assert!(!list_line.contains("read:"));
    }

    #[test]
    fn test_build_skills_prompt_disabled() {
        let skills = vec![make_skill("github", "GitHub ops")];
        let prompt = build_skills_prompt(
            &skills,
            &["github".to_string()],
            false,
            &HashMap::new(),
            &SkillPromptBudget::default(),
            &[],
            &std::collections::HashSet::new(),
        );
        assert_eq!(prompt, "");
    }

    #[test]
    fn test_build_skills_prompt_disable_model_invocation() {
        let mut skill = make_skill("github", "GitHub ops");
        skill.disable_model_invocation = Some(true);
        let skills = vec![skill];
        let prompt = build_skills_prompt(
            &skills,
            &[],
            false,
            &HashMap::new(),
            &SkillPromptBudget::default(),
            &[],
            &std::collections::HashSet::new(),
        );
        assert_eq!(prompt, "");
    }

    #[test]
    fn test_build_skills_prompt_compact_fallback() {
        // Create skills that would exceed a tiny budget in full format
        let mut skills = Vec::new();
        for i in 0..50 {
            skills.push(make_skill_with_path(
                &format!("skill_{}", i),
                &format!("A very long description for skill number {} that takes up lots of space in the prompt", i),
                &format!("/home/user/skills/skill_{}/SKILL.md", i),
            ));
        }
        let budget = SkillPromptBudget {
            max_count: 150,
            max_chars: 2000, // Very small budget to force compact
            max_file_bytes: DEFAULT_MAX_SKILL_FILE_BYTES,
            max_candidates_per_root: DEFAULT_MAX_CANDIDATES_PER_ROOT,
        };
        let prompt = build_skills_prompt(
            &skills,
            &[],
            false,
            &HashMap::new(),
            &budget,
            &[],
            &std::collections::HashSet::new(),
        );
        // Should either fall back to compact format (just `- name` per skill),
        // emit a truncation warning, or be empty when even the header doesn't
        // fit. In all three cases the result must not exceed the budget
        // materially (allowing the 120-char warning headroom).
        assert!(prompt.len() <= budget.max_chars + 200);
        if !prompt.is_empty() {
            assert!(prompt.contains("skill` tool"));
        }
    }

    #[test]
    fn test_build_skills_prompt_bundled_allowlist() {
        let mut skill1 = make_skill("github", "GitHub ops");
        skill1.source = "bundled".to_string();
        let mut skill2 = make_skill("slack", "Slack ops");
        skill2.source = "bundled".to_string();
        let skill3 = make_skill("custom", "Custom ops"); // source: "managed"
        let skills = vec![skill1, skill2, skill3];

        // Only allow "github" from bundled
        let prompt = build_skills_prompt(
            &skills,
            &[],
            false,
            &HashMap::new(),
            &SkillPromptBudget::default(),
            &["github".to_string()],
            &std::collections::HashSet::new(),
        );
        assert!(prompt.contains("github"));
        assert!(!prompt.contains("slack")); // blocked by allowlist
        assert!(prompt.contains("custom")); // non-bundled, always allowed
    }

    #[test]
    fn test_build_skills_prompt_env_check_no_requires() {
        // Skill with no requires should always pass env_check
        let skills = vec![make_skill("basic", "A basic skill")];
        let prompt = build_skills_prompt(
            &skills,
            &[],
            true,
            &HashMap::new(),
            &SkillPromptBudget::default(),
            &[],
            &std::collections::HashSet::new(),
        );
        assert!(prompt.contains("basic"));
    }

    #[test]
    fn test_check_requirements_empty() {
        // Empty requirements always pass
        assert!(check_requirements(&SkillRequires::default(), None));
    }

    #[test]
    fn test_check_requirements_always() {
        let req = SkillRequires {
            always: true,
            bins: vec!["nonexistent_binary_abc_xyz".to_string()],
            ..Default::default()
        };
        // always=true should pass even with nonexistent binary
        assert!(check_requirements(&req, None));
    }

    #[test]
    fn test_check_requirements_any_bins_pass() {
        // git should exist on most systems
        let req = SkillRequires {
            any_bins: vec!["nonexistent_abc_xyz".to_string(), "sh".to_string()],
            ..Default::default()
        };
        // "sh" should exist, so OR logic passes
        assert!(check_requirements(&req, None));
    }

    #[test]
    fn test_check_requirements_any_bins_fail() {
        let req = SkillRequires {
            any_bins: vec![
                "nonexistent_abc_1".to_string(),
                "nonexistent_abc_2".to_string(),
            ],
            ..Default::default()
        };
        assert!(!check_requirements(&req, None));
    }

    #[test]
    fn test_check_requirements_wrong_os() {
        let req = SkillRequires {
            os: vec!["nonexistent-os-xyz".to_string()],
            ..Default::default()
        };
        assert!(!check_requirements(&req, None));
    }

    #[test]
    fn test_check_requirements_with_configured_env() {
        let req = SkillRequires {
            env: vec!["MY_TEST_KEY_XYZ".to_string()],
            ..Default::default()
        };
        // Without configured env, should fail (assuming MY_TEST_KEY_XYZ is not set)
        assert!(!check_requirements(&req, None));
        // With configured env, should pass
        let mut configured = HashMap::new();
        configured.insert("MY_TEST_KEY_XYZ".to_string(), "some-value".to_string());
        assert!(check_requirements(&req, Some(&configured)));
        // Empty value should still fail
        configured.insert("MY_TEST_KEY_XYZ".to_string(), String::new());
        assert!(!check_requirements(&req, Some(&configured)));
    }

    #[test]
    fn test_check_requirements_primary_env() {
        let req = SkillRequires {
            env: vec!["MY_API_KEY".to_string()],
            primary_env: Some("MY_API_KEY".to_string()),
            ..Default::default()
        };
        // With apiKey configured via __apiKey__, primary_env should be satisfied
        let mut configured = HashMap::new();
        configured.insert("__apiKey__".to_string(), "sk-test-123".to_string());
        assert!(check_requirements(&req, Some(&configured)));
    }

    #[test]
    fn test_compact_path() {
        // Can't test exact home dir, but test the no-change case
        assert_eq!(compact_path("/usr/local/bin/tool"), "/usr/local/bin/tool");
        // Path without home prefix stays unchanged
        assert_eq!(compact_path("/etc/config"), "/etc/config");
    }

    #[test]
    fn test_normalize_skill_command_name() {
        assert_eq!(normalize_skill_command_name("github"), "github");
        assert_eq!(normalize_skill_command_name("my-skill"), "my_skill");
        assert_eq!(
            normalize_skill_command_name("My Cool Skill!"),
            "my_cool_skill"
        );
        assert_eq!(normalize_skill_command_name("---test---"), "test");
        assert_eq!(normalize_skill_command_name(""), "skill");
        // Long name truncation
        let long = "a".repeat(50);
        assert_eq!(normalize_skill_command_name(&long).len(), 32);
    }

    #[test]
    fn test_mask_value() {
        assert_eq!(mask_value(""), "");
        assert_eq!(mask_value("short"), "****");
        assert_eq!(mask_value("12345678"), "****");
        assert_eq!(mask_value("123456789"), "1234...6789");
        assert_eq!(mask_value("sk-abcdefghijklmnop"), "sk-a...mnop");
    }

    #[test]
    fn test_is_masked_value() {
        assert!(is_masked_value("****"));
        assert!(is_masked_value("1234...6789"));
        assert!(!is_masked_value("real-value"));
        assert!(!is_masked_value(""));
    }

    #[test]
    fn test_unquote() {
        assert_eq!(unquote("\"hello\""), "hello");
        assert_eq!(unquote("'world'"), "world");
        assert_eq!(unquote("plain"), "plain");
    }

    #[test]
    fn test_check_requirements_detail() {
        let req = SkillRequires {
            bins: vec!["nonexistent_bin_xyz".to_string()],
            any_bins: vec!["nonexistent_a".to_string(), "nonexistent_b".to_string()],
            env: vec!["NONEXISTENT_ENV_XYZ".to_string()],
            ..Default::default()
        };
        let detail = check_requirements_detail(&req, None);
        assert!(!detail.eligible);
        assert_eq!(detail.missing_bins, vec!["nonexistent_bin_xyz"]);
        assert_eq!(
            detail.missing_any_bins,
            vec!["nonexistent_a", "nonexistent_b"]
        );
        assert_eq!(detail.missing_env, vec!["NONEXISTENT_ENV_XYZ"]);
    }

    #[test]
    fn test_check_requirements_detail_always() {
        let req = SkillRequires {
            always: true,
            bins: vec!["nonexistent_bin_xyz".to_string()],
            ..Default::default()
        };
        let detail = check_requirements_detail(&req, None);
        assert!(detail.eligible);
        assert!(detail.missing_bins.is_empty());
    }

    #[test]
    fn test_health_check() {
        let skills = vec![
            make_skill("ok-skill", "passes"),
            make_skill("disabled-skill", "disabled"),
        ];
        let disabled = vec!["disabled-skill".to_string()];
        let statuses = check_all_skills_status(&skills, &disabled, false, &HashMap::new(), &[]);
        assert_eq!(statuses.len(), 2);
        assert!(statuses[0].eligible);
        assert!(!statuses[0].disabled);
        assert!(!statuses[1].eligible);
        assert!(statuses[1].disabled);
    }

    #[test]
    fn test_parse_bool_value() {
        assert_eq!(parse_bool_value("true"), Some(true));
        assert_eq!(parse_bool_value("yes"), Some(true));
        assert_eq!(parse_bool_value("false"), Some(false));
        assert_eq!(parse_bool_value("no"), Some(false));
        assert_eq!(parse_bool_value("invalid"), None);
    }

    #[test]
    fn test_skill_cache_version() {
        let v1 = crate::skills::skill_cache_version();
        crate::skills::bump_skill_version();
        let v2 = crate::skills::skill_cache_version();
        assert!(v2 > v1);
    }
}
