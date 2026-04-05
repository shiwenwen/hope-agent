use super::types::*;

// ── Frontmatter Parsing ──────────────────────────────────────────

/// Parsed frontmatter result with all extended fields.
pub(super) struct ParsedFrontmatter {
    pub name: String,
    pub description: String,
    pub requires: SkillRequires,
    #[allow(dead_code)]
    pub body: String,
    pub skill_key: Option<String>,
    pub user_invocable: Option<bool>,
    pub disable_model_invocation: Option<bool>,
    pub command_dispatch: Option<String>,
    pub command_tool: Option<String>,
    pub command_arg_mode: Option<String>,
    pub command_arg_placeholder: Option<String>,
    pub command_arg_options: Option<Vec<String>>,
    pub command_prompt_template: Option<String>,
    pub install: Vec<SkillInstallSpec>,
    pub allowed_tools: Vec<String>,
    pub context_mode: Option<String>,
}

/// Extract YAML frontmatter from a SKILL.md file content.
pub(super) fn parse_frontmatter(content: &str) -> Option<ParsedFrontmatter> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return None;
    }
    // Find the closing ---
    let after_opening = &trimmed[3..];
    let end_idx = after_opening.find("\n---")?;
    let yaml_block = &after_opening[..end_idx];
    let body = &after_opening[end_idx + 4..]; // skip \n---

    let mut name: Option<String> = None;
    let mut description: Option<String> = None;
    let mut skill_key: Option<String> = None;
    let mut user_invocable: Option<bool> = None;
    let mut disable_model_invocation: Option<bool> = None;
    let mut command_dispatch: Option<String> = None;
    let mut command_tool: Option<String> = None;
    let mut command_arg_mode: Option<String> = None;
    let mut command_arg_placeholder: Option<String> = None;
    let mut command_arg_options: Option<Vec<String>> = None;
    let mut command_prompt_template: Option<String> = None;
    let mut allowed_tools: Vec<String> = Vec::new();
    let mut context_mode: Option<String> = None;

    let requires = parse_requires(yaml_block);
    let install = parse_install_specs(yaml_block);

    for line in yaml_block.lines() {
        let line_trimmed = line.trim();
        // Only parse root-level keys (no indentation)
        let indent = line.len() - line.trim_start().len();
        if indent > 0 {
            continue;
        }
        if let Some(rest) = line_trimmed.strip_prefix("name:") {
            name = Some(unquote(rest.trim()));
        } else if let Some(rest) = line_trimmed.strip_prefix("description:") {
            description = Some(unquote(rest.trim()));
        } else if let Some(rest) = line_trimmed
            .strip_prefix("skillKey:")
            .or_else(|| line_trimmed.strip_prefix("skill_key:"))
        {
            skill_key = Some(unquote(rest.trim()));
        } else if let Some(rest) = line_trimmed
            .strip_prefix("user-invocable:")
            .or_else(|| line_trimmed.strip_prefix("user_invocable:"))
        {
            user_invocable = parse_bool_value(rest.trim());
        } else if let Some(rest) = line_trimmed
            .strip_prefix("disable-model-invocation:")
            .or_else(|| line_trimmed.strip_prefix("disable_model_invocation:"))
        {
            disable_model_invocation = parse_bool_value(rest.trim());
        } else if let Some(rest) = line_trimmed
            .strip_prefix("command-dispatch:")
            .or_else(|| line_trimmed.strip_prefix("command_dispatch:"))
        {
            command_dispatch = Some(unquote(rest.trim()));
        } else if let Some(rest) = line_trimmed
            .strip_prefix("command-tool:")
            .or_else(|| line_trimmed.strip_prefix("command_tool:"))
        {
            command_tool = Some(unquote(rest.trim()));
        } else if let Some(rest) = line_trimmed
            .strip_prefix("command-arg-mode:")
            .or_else(|| line_trimmed.strip_prefix("command_arg_mode:"))
        {
            command_arg_mode = Some(unquote(rest.trim()));
        } else if let Some(rest) = line_trimmed
            .strip_prefix("command-arg-placeholder:")
            .or_else(|| line_trimmed.strip_prefix("command_arg_placeholder:"))
        {
            command_arg_placeholder = Some(unquote(rest.trim()));
        } else if let Some(rest) = line_trimmed
            .strip_prefix("command-arg-options:")
            .or_else(|| line_trimmed.strip_prefix("command_arg_options:"))
        {
            command_arg_options = parse_inline_string_array(rest.trim());
        } else if let Some(rest) = line_trimmed
            .strip_prefix("command-prompt-template:")
            .or_else(|| line_trimmed.strip_prefix("command_prompt_template:"))
        {
            let val = unquote(rest.trim());
            if !val.is_empty() {
                command_prompt_template = Some(val);
            }
        } else if let Some(rest) = line_trimmed
            .strip_prefix("allowed-tools:")
            .or_else(|| line_trimmed.strip_prefix("allowed_tools:"))
        {
            if let Some(arr) = parse_inline_string_array(rest.trim()) {
                allowed_tools = arr;
            }
        } else if let Some(rest) = line_trimmed.strip_prefix("context:") {
            let val = unquote(rest.trim());
            if !val.is_empty() {
                context_mode = Some(val);
            }
        }
    }

    let name = name.filter(|n| !n.is_empty())?;
    let description = description.unwrap_or_default();

    // For "prompt" dispatch, use body as template if no explicit template was set
    if command_dispatch.as_deref() == Some("prompt") && command_prompt_template.is_none() {
        let body_trimmed = body.trim();
        if !body_trimmed.is_empty() {
            command_prompt_template = Some(body_trimmed.to_string());
        }
    }

    Some(ParsedFrontmatter {
        name,
        description,
        requires,
        body: body.to_string(),
        skill_key,
        user_invocable,
        disable_model_invocation,
        command_dispatch,
        command_tool,
        command_arg_mode,
        command_arg_placeholder,
        command_arg_options,
        command_prompt_template,
        install,
        allowed_tools,
        context_mode,
    })
}

/// Parse a boolean-ish YAML value.
pub(super) fn parse_bool_value(s: &str) -> Option<bool> {
    let s = unquote(s);
    let lower = s.to_lowercase();
    match lower.as_str() {
        "true" | "yes" | "1" | "on" => Some(true),
        "false" | "no" | "0" | "off" => Some(false),
        _ => None,
    }
}

/// Parse the `requires:` block from a YAML frontmatter string.
/// Supports both inline arrays `[a, b]` and list style `- item`.
pub(super) fn parse_requires(yaml_block: &str) -> SkillRequires {
    let mut req = SkillRequires::default();
    let mut in_requires = false;
    let mut current_key = String::new();

    for line in yaml_block.lines() {
        if line.trim().is_empty() || line.trim().starts_with('#') {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        let trimmed = line.trim();

        if indent == 0 {
            // Root-level key
            if trimmed == "requires:" || trimmed.starts_with("requires:") {
                in_requires = true;
                // Check for inline value after "requires:"
                if let Some(rest) = trimmed.strip_prefix("requires:") {
                    let rest = rest.trim();
                    // Handle root-level simple keys like "always: true"
                    if !rest.is_empty() && !rest.starts_with('{') {
                        // Not a block, skip
                    }
                }
            } else {
                in_requires = false;
            }
            current_key.clear();
            continue;
        }

        if !in_requires {
            continue;
        }

        if indent >= 2 && indent < 4 {
            // Sub-key of requires (e.g., "bins:", "env:", "os:", "anyBins:", "config:")
            if let Some((key, val)) = trimmed.split_once(':') {
                let key = key.trim();
                let val = val.trim();
                current_key = key.to_string();
                if !val.is_empty() {
                    // Inline array: bins: [git, gh]
                    let items = parse_yaml_inline_list(val);
                    push_requires_items(&mut req, key, items);
                }
            }
        } else if indent >= 4 {
            // List item: - git
            if let Some(item) = trimmed.strip_prefix("- ") {
                let item = unquote(item.trim()).to_string();
                if !item.is_empty() {
                    push_requires_items(&mut req, &current_key, vec![item]);
                }
            }
        }
    }

    // Parse root-level `always:` and `primaryEnv:` (outside requires block)
    for line in yaml_block.lines() {
        let indent = line.len() - line.trim_start().len();
        if indent != 0 {
            continue;
        }
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("always:") {
            if let Some(v) = parse_bool_value(rest.trim()) {
                req.always = v;
            }
        } else if let Some(rest) = trimmed
            .strip_prefix("primaryEnv:")
            .or_else(|| trimmed.strip_prefix("primary_env:"))
        {
            let val = unquote(rest.trim());
            if !val.is_empty() {
                req.primary_env = Some(val);
            }
        }
    }

    req
}

/// Parse a YAML inline list like `[git, gh]` or `["git", "gh"]`.
fn parse_yaml_inline_list(s: &str) -> Vec<String> {
    let s = s.trim();
    if s.starts_with('[') && s.ends_with(']') {
        let inner = &s[1..s.len() - 1];
        inner
            .split(',')
            .map(|item| unquote(item.trim()).to_string())
            .filter(|s| !s.is_empty())
            .collect()
    } else {
        Vec::new()
    }
}

fn push_requires_items(req: &mut SkillRequires, key: &str, items: Vec<String>) {
    match key {
        "bins" => req.bins.extend(items),
        "anyBins" | "any_bins" => req.any_bins.extend(items),
        "env" => req.env.extend(items),
        "os" => req.os.extend(items),
        "config" => req.config.extend(items),
        _ => {}
    }
}

/// Parse the `install:` block from YAML frontmatter.
/// Supports list of install specs with kind/formula/package/module/bins/label/os.
pub(super) fn parse_install_specs(yaml_block: &str) -> Vec<SkillInstallSpec> {
    let mut specs: Vec<SkillInstallSpec> = Vec::new();
    let mut in_install = false;
    let mut current_spec: Option<InstallSpecBuilder> = None;

    for line in yaml_block.lines() {
        if line.trim().is_empty() || line.trim().starts_with('#') {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        let trimmed = line.trim();

        if indent == 0 {
            if trimmed == "install:" {
                in_install = true;
                // Flush any pending spec
                if let Some(builder) = current_spec.take() {
                    if let Some(spec) = builder.build() {
                        specs.push(spec);
                    }
                }
            } else {
                if in_install {
                    // Flush pending spec when leaving install block
                    if let Some(builder) = current_spec.take() {
                        if let Some(spec) = builder.build() {
                            specs.push(spec);
                        }
                    }
                }
                in_install = false;
            }
            continue;
        }

        if !in_install {
            continue;
        }

        // List item start: "- kind: brew" or "- kind: node"
        if indent == 2 && trimmed.starts_with("- ") {
            // Flush previous spec
            if let Some(builder) = current_spec.take() {
                if let Some(spec) = builder.build() {
                    specs.push(spec);
                }
            }
            let rest = &trimmed[2..];
            let mut builder = InstallSpecBuilder::default();
            if let Some((key, val)) = rest.split_once(':') {
                builder.set(key.trim(), val.trim());
            }
            current_spec = Some(builder);
        } else if indent >= 4 {
            // Continuation of current spec
            if let Some(ref mut builder) = current_spec {
                if let Some((key, val)) = trimmed.split_once(':') {
                    builder.set(key.trim(), val.trim());
                }
            }
        }
    }

    // Flush last spec
    if let Some(builder) = current_spec.take() {
        if let Some(spec) = builder.build() {
            specs.push(spec);
        }
    }

    specs
}

#[derive(Default)]
struct InstallSpecBuilder {
    kind: Option<String>,
    formula: Option<String>,
    package: Option<String>,
    go_module: Option<String>,
    bins: Vec<String>,
    label: Option<String>,
    os: Vec<String>,
}

impl InstallSpecBuilder {
    fn set(&mut self, key: &str, val: &str) {
        let val = unquote(val);
        match key {
            "kind" => self.kind = Some(val),
            "formula" => self.formula = Some(val),
            "package" => self.package = Some(val),
            "module" => self.go_module = Some(val),
            "label" => self.label = Some(val),
            "bins" => {
                self.bins = parse_yaml_inline_list(&val)
                    .into_iter()
                    .filter(|s| !s.is_empty())
                    .collect()
            }
            "os" => {
                self.os = parse_yaml_inline_list(&val)
                    .into_iter()
                    .filter(|s| !s.is_empty())
                    .collect()
            }
            _ => {}
        }
    }

    fn build(self) -> Option<SkillInstallSpec> {
        let kind = self.kind?;
        // Validate kind
        match kind.as_str() {
            "brew" | "node" | "go" | "uv" | "download" => {}
            _ => return None,
        }
        Some(SkillInstallSpec {
            kind,
            formula: self.formula,
            package: self.package,
            go_module: self.go_module,
            bins: self.bins,
            label: self.label,
            os: self.os,
        })
    }
}

/// Parse an inline YAML array like `[opt1, opt2, "opt 3"]` into `Some(Vec<String>)`.
/// Returns `None` if the input doesn't look like an array.
fn parse_inline_string_array(s: &str) -> Option<Vec<String>> {
    let s = s.trim();
    let inner = s.strip_prefix('[')?.strip_suffix(']')?;
    let items: Vec<String> = inner
        .split(',')
        .map(|item| unquote(item.trim()))
        .filter(|item| !item.is_empty())
        .collect();
    if items.is_empty() {
        None
    } else {
        Some(items)
    }
}

pub(super) fn unquote(s: &str) -> String {
    let s = s.trim();
    if (s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')) {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}
