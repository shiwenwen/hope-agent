use super::types::*;

// ── Frontmatter Parsing ──────────────────────────────────────────

/// Parsed frontmatter result with all extended fields.
pub(super) struct ParsedFrontmatter {
    pub name: String,
    pub description: String,
    /// See `SkillEntry::when_to_use`.
    pub when_to_use: Option<String>,
    pub requires: SkillRequires,
    #[allow(dead_code)]
    pub body: String,
    pub skill_key: Option<String>,
    /// See `SkillEntry::aliases`.
    pub aliases: Vec<String>,
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
    /// Agent id to use when running a `context: fork` skill in a sub-agent.
    /// When unset the sub-agent inherits the parent agent. Must resolve via
    /// `agent_loader::load_agent`; invalid ids log a warning and fall back.
    pub agent: Option<String>,
    /// Reasoning / thinking effort forwarded to the provider at fork time.
    /// Accepts the same shorthand as `agent::config::clamp_reasoning_effort`:
    /// `low | medium | high | xhigh | none`.
    pub effort: Option<String>,
    /// Gitignore-style path patterns that gate catalog visibility. When set,
    /// the skill is hidden from the system prompt until the session touches
    /// a matching file (via read/write/edit/apply_patch), at which point it
    /// is activated and kept in the catalog for the remainder of the session.
    /// `None` = always visible (unchanged behavior for skills without paths).
    pub paths: Option<Vec<String>>,
    pub status: SkillStatus,
    pub authored_by: Option<String>,
    pub rationale: Option<String>,
    /// Display-only metadata aggregated from top-level YAML and vendor-
    /// namespaced `metadata.openclaw` / `metadata.hermes` blocks. See
    /// [`crate::skills::SkillDisplay`].
    pub display: SkillDisplay,
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
    let mut aliases: Vec<String> = Vec::new();
    let mut when_to_use: Option<String> = None;
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
    let mut agent: Option<String> = None;
    let mut effort: Option<String> = None;
    let mut paths: Option<Vec<String>> = None;
    let mut status: SkillStatus = SkillStatus::Active;
    let mut authored_by: Option<String> = None;
    let mut rationale: Option<String> = None;
    // Display-only top-level fields. Lifted into `SkillDisplay` after the loop,
    // merged with anything pulled from `metadata.<vendor>` namespaces.
    let mut top_version: Option<String> = None;
    let mut top_license: Option<String> = None;
    let mut top_author: Option<String> = None;

    let mut requires = parse_requires(yaml_block);
    let mut install = parse_install_specs(yaml_block);
    let mut nested = parse_metadata_namespaces(yaml_block);

    // Lift vendor-namespaced requires/install into top-level when the user
    // didn't set the canonical fields. This makes vendored OW skills work
    // out-of-the-box without losing dependency-check info.
    if requires.is_empty() {
        if let Some(req) = nested.openclaw_requires.take() {
            requires = req;
        } else if let Some(req) = nested.hermes_requires.take() {
            requires = req;
        }
    }
    if install.is_empty() {
        if !nested.openclaw_install.is_empty() {
            install = std::mem::take(&mut nested.openclaw_install);
        } else if !nested.hermes_install.is_empty() {
            install = std::mem::take(&mut nested.hermes_install);
        }
    }

    let lines: Vec<&str> = yaml_block.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let line_trimmed = line.trim();
        // Only parse root-level keys (no indentation)
        let indent = line.len() - line.trim_start().len();
        if indent > 0 || line_trimmed.is_empty() {
            i += 1;
            continue;
        }

        // String-typed fields. Each candidate key produces a single-line value
        // unless it ends with `>` (folded block scalar — newlines collapse to
        // spaces) or `|` (literal block scalar — newlines preserved); both are
        // followed by an indented continuation block. See Hermes Agent /
        // Anthropic SKILL.md authoring conventions which lean heavily on `>`
        // for long descriptions.
        if let Some(rest) = line_trimmed.strip_prefix("name:") {
            // `name` is always single-line — keep simple.
            name = Some(unquote(rest.trim()));
            i += 1;
            continue;
        }
        if let Some((value, consumed)) =
            read_string_field(&lines, i, line_trimmed, &["description:"])
        {
            description = Some(value);
            i += consumed;
            continue;
        }
        if let Some((value, consumed)) = read_string_field(
            &lines,
            i,
            line_trimmed,
            &["whenToUse:", "when-to-use:", "when_to_use:"],
        ) {
            if !value.is_empty() {
                when_to_use = Some(value);
            }
            i += consumed;
            continue;
        }
        if let Some((value, consumed)) = read_string_field(&lines, i, line_trimmed, &["rationale:"])
        {
            if !value.is_empty() {
                rationale = Some(value);
            }
            i += consumed;
            continue;
        }
        if let Some((value, consumed)) = read_string_field(
            &lines,
            i,
            line_trimmed,
            &["command-prompt-template:", "command_prompt_template:"],
        ) {
            if !value.is_empty() {
                command_prompt_template = Some(value);
            }
            i += consumed;
            continue;
        }

        // List-typed fields. Each accepts inline `[a, b]` or block `- a\n  - b`
        // syntax. The hand-rolled parser previously only honored inline; the
        // block form is what Hermes vendors use heavily.
        if let Some((items, consumed)) = read_list_field(&lines, i, line_trimmed, &["aliases:"]) {
            aliases = items;
            i += consumed;
            continue;
        }
        if let Some((items, consumed)) = read_list_field(&lines, i, line_trimmed, &["paths:"]) {
            if !items.is_empty() {
                paths = Some(items);
            }
            i += consumed;
            continue;
        }
        if let Some((items, consumed)) = read_list_field(
            &lines,
            i,
            line_trimmed,
            &["allowed-tools:", "allowed_tools:"],
        ) {
            allowed_tools = items;
            i += consumed;
            continue;
        }
        if let Some((items, consumed)) = read_list_field(
            &lines,
            i,
            line_trimmed,
            &["command-arg-options:", "command_arg_options:"],
        ) {
            command_arg_options = if items.is_empty() { None } else { Some(items) };
            i += consumed;
            continue;
        }

        // Single-line value fields. The remaining keys never legitimately
        // appear as block scalars; treat the rest of the line as the value.
        if let Some(rest) = line_trimmed
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
            // Anthropic-common spelling — accept as alias.
            .or_else(|| line_trimmed.strip_prefix("argumentHint:"))
            .or_else(|| line_trimmed.strip_prefix("argument-hint:"))
            .or_else(|| line_trimmed.strip_prefix("argument_hint:"))
        {
            command_arg_placeholder = Some(unquote(rest.trim()));
        } else if let Some(rest) = line_trimmed.strip_prefix("context:") {
            let val = unquote(rest.trim());
            if !val.is_empty() {
                context_mode = Some(val);
            }
        } else if let Some(rest) = line_trimmed.strip_prefix("agent:") {
            let val = unquote(rest.trim());
            if !val.is_empty() {
                agent = Some(val);
            }
        } else if let Some(rest) = line_trimmed.strip_prefix("effort:") {
            let val = unquote(rest.trim());
            if !val.is_empty() {
                effort = Some(val);
            }
        } else if let Some(rest) = line_trimmed.strip_prefix("status:") {
            let val = unquote(rest.trim());
            if !val.is_empty() {
                status = SkillStatus::from_str(&val);
            }
        } else if let Some(rest) = line_trimmed
            .strip_prefix("authored-by:")
            .or_else(|| line_trimmed.strip_prefix("authored_by:"))
        {
            let val = unquote(rest.trim());
            if !val.is_empty() {
                authored_by = Some(val);
            }
        } else if let Some(rest) = line_trimmed.strip_prefix("version:") {
            // Display-only top-level field used by Hermes / Anthropic skills.
            let val = unquote(rest.trim());
            if !val.is_empty() {
                top_version = Some(val);
            }
        } else if let Some(rest) = line_trimmed.strip_prefix("license:") {
            // Display-only. Surfaced as a badge so users see "Proprietary"
            // skills (Anthropic marketplace) clearly distinguished from MIT.
            let val = unquote(rest.trim());
            if !val.is_empty() {
                top_license = Some(val);
            }
        } else if let Some(rest) = line_trimmed.strip_prefix("author:") {
            // Display-only. Distinct from `authored_by` (which is HA's
            // internal "user vs auto-review" provenance flag).
            let val = unquote(rest.trim());
            if !val.is_empty() {
                top_author = Some(val);
            }
        }
        i += 1;
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

    // Assemble display from top-level fields + vendor-namespaced metadata.
    // Top-level wins; vendor namespaces fill in only when top-level is absent.
    let mut display = SkillDisplay {
        emoji: nested.openclaw_emoji.or(nested.hermes_emoji),
        version: top_version,
        license: top_license,
        license_label: None,
        is_proprietary: false,
        author: top_author,
        tags: nested.hermes_tags,
        related_skills: nested.hermes_related_skills,
    };
    display.finalize();

    Some(ParsedFrontmatter {
        name,
        description,
        when_to_use,
        requires,
        body: body.to_string(),
        aliases,
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
        agent,
        effort,
        paths,
        status,
        authored_by,
        rationale,
        display,
    })
}

/// Try to parse a string-typed root-level field starting at `lines[i]`.
///
/// Returns `Some((value, consumed))` when one of `keys` matches the line, where
/// `consumed` is the number of lines (>= 1) the field spans:
///
/// - Inline (`key: value`) → consumes 1 line.
/// - Folded block scalar (`key: >`) → joins continuation lines with spaces.
/// - Literal block scalar (`key: |`) → joins continuation lines with `\n`.
///
/// Continuation lines are any indented or empty lines following the key; the
/// block ends at the next root-level non-empty line. Mirrors the YAML 1.2
/// rules just enough to handle real-world SKILL.md vendored from Hermes
/// Agent / Anthropic, without pulling in a full YAML library.
fn read_string_field(
    lines: &[&str],
    i: usize,
    line_trimmed: &str,
    keys: &[&str],
) -> Option<(String, usize)> {
    let rest = keys.iter().find_map(|k| line_trimmed.strip_prefix(k))?;
    let val_after_colon = rest.trim();
    if val_after_colon == ">" || val_after_colon == "|" {
        let folded = val_after_colon == ">";
        let (text, consumed) = read_continuation_block(lines, i + 1);
        let joined = if folded {
            text.into_iter().collect::<Vec<_>>().join(" ")
        } else {
            text.into_iter().collect::<Vec<_>>().join("\n")
        };
        Some((joined.trim().to_string(), 1 + consumed))
    } else {
        Some((unquote(val_after_colon), 1))
    }
}

/// Try to parse a list-typed root-level field starting at `lines[i]`.
///
/// Returns `Some((items, consumed))` when one of `keys` matches the line:
///
/// - Inline `[a, b, c]` → consumes 1 line.
/// - Empty value followed by `  - item` block list → consumes the block.
/// - Empty value with no continuation → returns `Some((vec![], 1))` so the
///   caller knows the key was seen (and decides whether to treat empty as
///   "unset" or "explicitly empty").
fn read_list_field(
    lines: &[&str],
    i: usize,
    line_trimmed: &str,
    keys: &[&str],
) -> Option<(Vec<String>, usize)> {
    let rest = keys.iter().find_map(|k| line_trimmed.strip_prefix(k))?;
    let val_after_colon = rest.trim();
    if val_after_colon.is_empty() {
        // Block-list form: scan indented `- item` continuation lines.
        let mut items = Vec::new();
        let mut consumed = 0;
        let mut j = i + 1;
        while j < lines.len() {
            let cont = lines[j];
            let cont_trimmed = cont.trim();
            let cont_indent = cont.len() - cont.trim_start().len();
            if cont_indent == 0 && !cont_trimmed.is_empty() {
                break;
            }
            if let Some(item) = cont_trimmed.strip_prefix("- ") {
                let v = unquote(item.trim());
                if !v.is_empty() {
                    items.push(v);
                }
            }
            consumed += 1;
            j += 1;
        }
        Some((items, 1 + consumed))
    } else if let Some(arr) = parse_inline_string_array(val_after_colon) {
        Some((arr, 1))
    } else {
        // Bare value like `paths: foo` is malformed; treat as no list to
        // preserve the existing parser's behavior.
        Some((Vec::new(), 1))
    }
}

/// Collect indented / blank continuation lines starting at `start`. Returns
/// the trimmed payload of each non-empty line and the number of lines
/// consumed (including any trailing blank lines that belong to the block).
fn read_continuation_block(lines: &[&str], start: usize) -> (Vec<String>, usize) {
    let mut payload = Vec::new();
    let mut consumed = 0;
    for j in start..lines.len() {
        let line = lines[j];
        let trimmed = line.trim();
        let indent = line.len() - line.trim_start().len();
        if indent == 0 && !trimmed.is_empty() {
            break;
        }
        if !trimmed.is_empty() {
            payload.push(trimmed.to_string());
        }
        consumed += 1;
    }
    (payload, consumed)
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

// ── Vendor-namespaced metadata blocks ─────────────────────────────
//
// OpenClaw / Hermes / Anthropic skills nest some fields under
// `metadata.<vendor>:`. We don't introduce a YAML library — the existing
// hand-rolled parser is intentionally permissive — but we do a second pass
// just for these namespaces so vendored skills don't lose their `requires`
// / `install` / `tags` info when imported via Quick Import.

#[derive(Debug, Default, Clone)]
pub(super) struct MetadataNamespaces {
    pub openclaw_requires: Option<SkillRequires>,
    pub openclaw_install: Vec<SkillInstallSpec>,
    pub openclaw_emoji: Option<String>,
    pub hermes_requires: Option<SkillRequires>,
    pub hermes_install: Vec<SkillInstallSpec>,
    pub hermes_emoji: Option<String>,
    pub hermes_tags: Vec<String>,
    pub hermes_related_skills: Vec<String>,
}

/// Parse `metadata.openclaw.*` and `metadata.hermes.*` blocks. The grammar
/// follows the existing parser's permissive style: indent-based, list items
/// with `- ` prefix or inline `[a, b]`, no full YAML support.
///
/// Layout the parser handles:
///
/// ```yaml
/// metadata:
///   openclaw:
///     emoji: "🐙"
///     requires:
///       bins: [gh]
///       anyBins:
///         - claude
///         - codex
///     install:
///       - kind: brew
///         formula: gh
///   hermes:
///     tags: [debugging, tdd]
///     related_skills:
///       - test-driven-development
/// ```
pub(super) fn parse_metadata_namespaces(yaml_block: &str) -> MetadataNamespaces {
    let mut out = MetadataNamespaces::default();

    // Cheap early-out — most SKILL.md files have no `metadata:` block at all,
    // so spare the full scan. The substring check is O(n) on the yaml block
    // (typically < 2 KB) and trips well below the regex/parser cost below.
    if !yaml_block.contains("metadata:") {
        return out;
    }

    let mut in_metadata = false;
    let mut current_vendor: Option<String> = None; // "openclaw" | "hermes"
    let mut current_subkey: Option<String> = None; // top-level under vendor
                                                   // For nested blocks like `requires` (which has its own sub-keys), and
                                                   // `install` (which is a list of objects). Keep one builder slot per
                                                   // namespace.
    let mut oc_install_builder: Option<InstallSpecBuilder> = None;
    let mut oc_install_specs: Vec<SkillInstallSpec> = Vec::new();
    let mut hr_install_builder: Option<InstallSpecBuilder> = None;
    let mut hr_install_specs: Vec<SkillInstallSpec> = Vec::new();

    for line in yaml_block.lines() {
        if line.trim().is_empty() || line.trim().starts_with('#') {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        let trimmed = line.trim();

        // Indent 0: top-level YAML keys.
        if indent == 0 {
            // Flush any in-flight install builders (closing the namespace).
            if let Some(b) = oc_install_builder.take() {
                if let Some(spec) = b.build() {
                    oc_install_specs.push(spec);
                }
            }
            if let Some(b) = hr_install_builder.take() {
                if let Some(spec) = b.build() {
                    hr_install_specs.push(spec);
                }
            }

            in_metadata = trimmed.starts_with("metadata:");
            current_vendor = None;
            current_subkey = None;
            continue;
        }

        if !in_metadata {
            continue;
        }

        // Indent 2: vendor key under `metadata:` — `openclaw:` / `hermes:`.
        if indent == 2 {
            if let Some(b) = oc_install_builder.take() {
                if let Some(spec) = b.build() {
                    oc_install_specs.push(spec);
                }
            }
            if let Some(b) = hr_install_builder.take() {
                if let Some(spec) = b.build() {
                    hr_install_specs.push(spec);
                }
            }
            current_subkey = None;
            if let Some((key, _)) = trimmed.split_once(':') {
                let key = key.trim();
                if key == "openclaw" || key == "hermes" {
                    current_vendor = Some(key.to_string());
                } else {
                    current_vendor = None;
                }
            } else {
                current_vendor = None;
            }
            continue;
        }

        let vendor = match current_vendor.as_deref() {
            Some(v) => v,
            None => continue,
        };

        // Indent 4: leaf or sub-block under vendor.
        if indent == 4 {
            if let Some(b) = oc_install_builder.take() {
                if let Some(spec) = b.build() {
                    oc_install_specs.push(spec);
                }
            }
            if let Some(b) = hr_install_builder.take() {
                if let Some(spec) = b.build() {
                    hr_install_specs.push(spec);
                }
            }
            current_subkey = None;

            if let Some((key, val)) = trimmed.split_once(':') {
                let key = key.trim();
                let val = val.trim();
                match key {
                    "emoji" => {
                        let v = unquote(val);
                        if !v.is_empty() {
                            match vendor {
                                "openclaw" => out.openclaw_emoji = Some(v),
                                "hermes" => out.hermes_emoji = Some(v),
                                _ => {}
                            }
                        }
                    }
                    "tags" if vendor == "hermes" => {
                        if !val.is_empty() {
                            let items = parse_yaml_inline_list(val);
                            if !items.is_empty() {
                                out.hermes_tags = items;
                            } else {
                                current_subkey = Some("tags".to_string());
                            }
                        } else {
                            current_subkey = Some("tags".to_string());
                        }
                    }
                    "related_skills" | "relatedSkills" if vendor == "hermes" => {
                        if !val.is_empty() {
                            let items = parse_yaml_inline_list(val);
                            if !items.is_empty() {
                                out.hermes_related_skills = items;
                            } else {
                                current_subkey = Some("related_skills".to_string());
                            }
                        } else {
                            current_subkey = Some("related_skills".to_string());
                        }
                    }
                    "requires" => {
                        // Marker — sub-keys parsed below at indent 6+.
                        current_subkey = Some("requires".to_string());
                    }
                    "install" => {
                        current_subkey = Some("install".to_string());
                    }
                    _ => {}
                }
            }
            continue;
        }

        // Indent ≥ 6: items inside a sub-block (`requires` / `install` / `tags` list).
        let sub = match current_subkey.as_deref() {
            Some(s) => s,
            None => continue,
        };

        match sub {
            "tags" if vendor == "hermes" => {
                if let Some(item) = trimmed.strip_prefix("- ") {
                    let v = unquote(item.trim());
                    if !v.is_empty() {
                        out.hermes_tags.push(v);
                    }
                }
            }
            "related_skills" if vendor == "hermes" => {
                if let Some(item) = trimmed.strip_prefix("- ") {
                    let v = unquote(item.trim());
                    if !v.is_empty() {
                        out.hermes_related_skills.push(v);
                    }
                }
            }
            // Inside the requires block. `sub` is "requires" while we're
            // looking at the very first sub-key, then transitions to
            // "requires:<key>" once we see e.g. `bins:` so subsequent list
            // items at indent 8 know which target list to push into.
            s if s == "requires" || s.starts_with("requires:") => {
                let req = match vendor {
                    "openclaw" => out
                        .openclaw_requires
                        .get_or_insert_with(SkillRequires::default),
                    "hermes" => out
                        .hermes_requires
                        .get_or_insert_with(SkillRequires::default),
                    _ => continue,
                };
                if indent == 6 {
                    // Sub-key transition (bins / anyBins / env / os / config).
                    if let Some((key, val)) = trimmed.split_once(':') {
                        let key = key.trim();
                        let val = val.trim();
                        if !val.is_empty() {
                            let items = parse_yaml_inline_list(val);
                            push_requires_items(req, key, items);
                        }
                        current_subkey = Some(format!("requires:{}", key));
                    }
                } else if indent >= 8 {
                    // List item under the currently-active requires sub-key.
                    if let Some(item) = trimmed.strip_prefix("- ") {
                        let key = current_subkey
                            .as_deref()
                            .and_then(|s| s.strip_prefix("requires:"))
                            .unwrap_or("");
                        let v = unquote(item.trim()).to_string();
                        if !v.is_empty() && !key.is_empty() {
                            push_requires_items(req, key, vec![v]);
                        }
                    }
                }
            }
            "install" => {
                let builder = match vendor {
                    "openclaw" => &mut oc_install_builder,
                    "hermes" => &mut hr_install_builder,
                    _ => continue,
                };
                let specs = match vendor {
                    "openclaw" => &mut oc_install_specs,
                    "hermes" => &mut hr_install_specs,
                    _ => continue,
                };

                // List item start: "- kind: brew"
                if indent == 6 && trimmed.starts_with("- ") {
                    if let Some(b) = builder.take() {
                        if let Some(spec) = b.build() {
                            specs.push(spec);
                        }
                    }
                    let rest = &trimmed[2..];
                    let mut nb = InstallSpecBuilder::default();
                    if let Some((key, val)) = rest.split_once(':') {
                        nb.set(key.trim(), val.trim());
                    }
                    *builder = Some(nb);
                } else if indent >= 8 {
                    if let Some(ref mut b) = builder {
                        if let Some((key, val)) = trimmed.split_once(':') {
                            b.set(key.trim(), val.trim());
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // Final flush.
    if let Some(b) = oc_install_builder.take() {
        if let Some(spec) = b.build() {
            oc_install_specs.push(spec);
        }
    }
    if let Some(b) = hr_install_builder.take() {
        if let Some(spec) = b.build() {
            hr_install_specs.push(spec);
        }
    }
    out.openclaw_install = oc_install_specs;
    out.hermes_install = hr_install_specs;

    out
}
