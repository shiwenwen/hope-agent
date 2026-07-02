use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Number, Value};

use crate::permission::{AskReason, Decision};
use crate::plan::{check_workflow_script_draft, GateReport, ScriptGateOptions};
use crate::session::SessionDB;
use crate::tools;

use super::runtime::{spawn_agent_tool_args, workflow_session_context, WorkflowSessionContext};
use super::types::WorkflowRun;

const MAX_PREVIEW_CALLS: usize = 128;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowPermissionPreview {
    pub summary: WorkflowPermissionPreviewSummary,
    pub calls: Vec<WorkflowPermissionPreviewCall>,
    pub truncated: bool,
}

impl WorkflowPermissionPreview {
    pub fn requires_user_approval(&self) -> bool {
        self.summary.ask > 0 || self.summary.dynamic > 0
    }

    pub fn has_denials(&self) -> bool {
        self.summary.deny > 0
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowPermissionPreviewSummary {
    pub total: usize,
    pub allow: usize,
    pub ask: usize,
    pub deny: usize,
    pub dynamic: usize,
    pub strict: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowPermissionPreviewCall {
    pub api: String,
    pub line: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    pub decision: String,
    pub strict: bool,
    pub dynamic: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowScriptPreview {
    pub gate: GateReport,
    pub gate_passed: bool,
    pub gate_feedback: String,
    pub permission: WorkflowPermissionPreview,
    pub can_create: bool,
    pub can_run_immediately: bool,
    pub requires_approval: bool,
    pub has_denials: bool,
}

pub fn preview_workflow_run(db: &SessionDB, run: &WorkflowRun) -> WorkflowPermissionPreview {
    let session_context = workflow_session_context(db, &run.session_id);
    preview_workflow_script(&run.script_source, &run.session_id, &session_context)
}

pub fn preview_workflow_script_for_session(
    db: &SessionDB,
    session_id: &str,
    script: &str,
    execution_mode: Option<&str>,
) -> WorkflowScriptPreview {
    let gate = check_workflow_script_draft(
        script,
        script_gate_options_for_execution_mode(execution_mode.unwrap_or("guarded")),
    );
    let gate_passed = gate.passed();
    let gate_feedback = gate.render_feedback("Workflow Script Gate");
    let session_context = workflow_session_context(db, session_id);
    let permission = preview_workflow_script(script, session_id, &session_context);
    let requires_approval = permission.requires_user_approval();
    let has_denials = permission.has_denials();
    let can_create = gate_passed && !has_denials;

    WorkflowScriptPreview {
        gate,
        gate_passed,
        gate_feedback,
        permission,
        can_create,
        can_run_immediately: can_create,
        requires_approval,
        has_denials,
    }
}

pub fn ensure_workflow_script_can_create(
    db: &SessionDB,
    session_id: &str,
    script: &str,
    execution_mode: Option<&str>,
) -> Result<WorkflowScriptPreview> {
    let preview = preview_workflow_script_for_session(db, session_id, script, execution_mode);
    if !preview.gate_passed {
        return Err(anyhow!(preview.gate_feedback.clone()));
    }
    if preview.has_denials {
        return Err(anyhow!(
            "Workflow permission preview denied; inspect the permission checklist before creating this run"
        ));
    }
    Ok(preview)
}

pub(crate) fn script_gate_options_for_execution_mode(execution_mode: &str) -> ScriptGateOptions {
    ScriptGateOptions {
        autonomous: execution_mode == "autonomous",
    }
}

pub(crate) fn preview_workflow_script(
    script: &str,
    session_id: &str,
    session_context: &WorkflowSessionContext,
) -> WorkflowPermissionPreview {
    let raw_calls = collect_raw_calls(script);
    let mut calls = Vec::new();
    let mut truncated = false;

    for raw in raw_calls {
        if calls.len() >= MAX_PREVIEW_CALLS {
            truncated = true;
            break;
        }
        preview_raw_call(raw, session_id, session_context, &mut calls);
    }

    let mut summary = WorkflowPermissionPreviewSummary {
        total: calls.len(),
        ..Default::default()
    };
    for call in &calls {
        match call.decision.as_str() {
            "allow" => summary.allow += 1,
            "ask" => summary.ask += 1,
            "deny" => summary.deny += 1,
            "dynamic" => summary.dynamic += 1,
            _ => {}
        }
        if call.strict {
            summary.strict += 1;
        }
    }

    WorkflowPermissionPreview {
        summary,
        calls,
        truncated,
    }
}

fn preview_raw_call(
    raw: RawWorkflowCall,
    session_id: &str,
    session_context: &WorkflowSessionContext,
    calls: &mut Vec<WorkflowPermissionPreviewCall>,
) {
    let Some(value) = raw.args else {
        if raw.api == "workflow.askUser" {
            preview_ask_user(raw.api, raw.line, None, session_id, calls);
            return;
        }
        if is_permission_neutral_api(&raw.api) {
            calls.push(allow_call(
                raw.api,
                raw.line,
                None,
                None,
                None,
                Some("permission-neutral workflow host API".to_string()),
            ));
            return;
        }
        calls.push(dynamic_call(
            raw.api,
            raw.line,
            "call arguments are not a static object literal",
        ));
        return;
    };
    match raw.api.as_str() {
        "workflow.tool" => preview_workflow_tool(raw.api, raw.line, value, session_context, calls),
        "workflow.read" => preview_tool_call(
            raw.api,
            raw.line,
            tools::TOOL_READ,
            normalize_tool_args(tools::TOOL_READ, value.clone()),
            optional_string(&value, "label"),
            session_context,
            calls,
        ),
        "workflow.grep" => preview_tool_call(
            raw.api,
            raw.line,
            tools::TOOL_GREP,
            normalize_tool_args(tools::TOOL_GREP, value.clone()),
            optional_string(&value, "label"),
            session_context,
            calls,
        ),
        "workflow.validate" => preview_validate(raw.api, raw.line, value, session_context, calls),
        "workflow.review" | "workflow.verify" => calls.push(allow_call(
            raw.api,
            raw.line,
            None,
            optional_string(&value, "label"),
            Some(value),
            Some("permission-neutral coding control-plane host API".to_string()),
        )),
        "workflow.spawnAgent" => {
            let label = optional_string(&value, "label");
            match spawn_agent_tool_args(&value) {
                Ok(args) => preview_tool_call(
                    raw.api,
                    raw.line,
                    tools::TOOL_SUBAGENT,
                    args,
                    label,
                    session_context,
                    calls,
                ),
                Err(err) => calls.push(dynamic_call(raw.api, raw.line, &err.to_string())),
            }
        }
        "workflow.askUser" => preview_ask_user(raw.api, raw.line, Some(value), session_id, calls),
        "workflow.fileSearch"
        | "workflow.diff"
        | "workflow.trace"
        | "workflow.task.create"
        | "workflow.task.update" => calls.push(allow_call(
            raw.api,
            raw.line,
            None,
            optional_string(&value, "label"),
            Some(value),
            None,
        )),
        _ => calls.push(dynamic_call(raw.api, raw.line, "unknown workflow host API")),
    }
}

fn preview_workflow_tool(
    api: String,
    line: usize,
    value: Value,
    session_context: &WorkflowSessionContext,
    calls: &mut Vec<WorkflowPermissionPreviewCall>,
) {
    let Some(name) = optional_string(&value, "name") else {
        calls.push(dynamic_call(
            api,
            line,
            "workflow.tool requires static name",
        ));
        return;
    };
    let args = value.get("args").cloned().unwrap_or_else(|| json!({}));
    let label = optional_string(&value, "label");
    preview_tool_call(
        api,
        line,
        &name,
        normalize_tool_args(&name, args),
        label,
        session_context,
        calls,
    );
}

fn preview_validate(
    api: String,
    line: usize,
    value: Value,
    session_context: &WorkflowSessionContext,
    calls: &mut Vec<WorkflowPermissionPreviewCall>,
) {
    let label = optional_string(&value, "label");
    match validation_exec_args(&value) {
        Ok(exec_args) => {
            for args in exec_args {
                preview_tool_call(
                    api.clone(),
                    line,
                    tools::TOOL_EXEC,
                    args,
                    label.clone(),
                    session_context,
                    calls,
                );
            }
        }
        Err(err) => calls.push(dynamic_call(api, line, &err.to_string())),
    }
}

fn preview_ask_user(
    api: String,
    line: usize,
    value: Option<Value>,
    session_id: &str,
    calls: &mut Vec<WorkflowPermissionPreviewCall>,
) {
    let label = value
        .as_ref()
        .and_then(|value| optional_string(value, "label"));
    match crate::permission::evaluate_approval_surface(Some(session_id)) {
        crate::permission::ApprovalSurface::Attended => calls.push(allow_call(
            api,
            line,
            None,
            label,
            value,
            Some("attended surface available".to_string()),
        )),
        crate::permission::ApprovalSurface::Unattended(reason) => {
            let action = crate::config::cached_config()
                .permission
                .unattended_approval_action;
            let (decision, detail) = match action {
                crate::permission::UnattendedApprovalAction::Proceed => ("allow", "proceed"),
                crate::permission::UnattendedApprovalAction::Deny => ("deny", "deny"),
            };
            calls.push(WorkflowPermissionPreviewCall {
                api,
                line,
                tool_name: None,
                decision: decision.to_string(),
                strict: false,
                dynamic: false,
                reason: Some(format!(
                    "askUser surface is unattended ({}) and runtime will apply unattendedApprovalAction={}: {}",
                    reason.as_str(),
                    detail,
                    reason.explain()
                )),
                label,
                args: value,
            });
        }
    }
}

fn is_permission_neutral_api(api: &str) -> bool {
    matches!(
        api,
        "workflow.fileSearch"
            | "workflow.diff"
            | "workflow.trace"
            | "workflow.task.create"
            | "workflow.task.update"
            | "workflow.askUser"
            | "workflow.review"
            | "workflow.verify"
    )
}

fn preview_tool_call(
    api: String,
    line: usize,
    tool_name: &str,
    args: Value,
    label: Option<String>,
    session_context: &WorkflowSessionContext,
    calls: &mut Vec<WorkflowPermissionPreviewCall>,
) {
    let decision = resolve_tool_preview(tool_name, &args, session_context);
    let (decision_label, strict, reason) = match decision {
        Decision::Allow => ("allow".to_string(), false, None),
        Decision::Ask { reason } => {
            let strict = reason.forbids_allow_always();
            ("ask".to_string(), strict, Some(render_ask_reason(&reason)))
        }
        Decision::Deny { reason } => ("deny".to_string(), false, Some(reason)),
    };
    calls.push(WorkflowPermissionPreviewCall {
        api,
        line,
        tool_name: Some(tool_name.to_string()),
        decision: decision_label,
        strict,
        dynamic: false,
        reason,
        label,
        args: Some(args),
    });
}

fn resolve_tool_preview(
    tool_name: &str,
    args: &Value,
    session_context: &WorkflowSessionContext,
) -> Decision {
    let default_path = session_context.working_dir.as_deref().unwrap_or(".");
    let resolve_ctx = crate::permission::engine::ResolveContext {
        tool_name,
        args,
        session_mode: session_context.session_mode,
        sandbox_mode: crate::permission::SandboxMode::Off,
        global_yolo: crate::security::dangerous::is_dangerous_skip_active(),
        plan_mode: false,
        plan_mode_allowed_tools: &[],
        plan_mode_ask_tools: &[],
        agent_custom_approval_enabled: false,
        agent_custom_approval_tools: &[],
        session_id: Some(&session_context.session_id),
        project_id: session_context.project_id.as_deref(),
        agent_id: session_context.agent_id.as_deref(),
        default_path: Some(default_path),
        is_internal_tool: tools::is_internal_tool(tool_name),
        smart_config: None,
        unattended: false,
        task_intent: None,
    };
    crate::permission::engine::resolve(&resolve_ctx)
}

fn render_ask_reason(reason: &AskReason) -> String {
    match reason {
        AskReason::EditTool => "edit-class tool requires approval".to_string(),
        AskReason::EditCommand { matched_pattern } => {
            format!("exec matched edit command pattern `{matched_pattern}`")
        }
        AskReason::DangerousCommand { matched_pattern } => {
            format!("exec matched dangerous command pattern `{matched_pattern}`")
        }
        AskReason::ProtectedPath { matched_path } => {
            format!("target path matches protected path `{matched_path}`")
        }
        AskReason::AgentCustomList => "agent custom approval list requires approval".to_string(),
        AskReason::SmartJudge { rationale } => format!("smart judge asked: {rationale}"),
        AskReason::BrowserEvaluate { script_preview } => {
            format!("browser control.evaluate JavaScript: {script_preview}")
        }
        AskReason::BrowserRawCdp { method } => {
            format!("browser raw CDP method `{method}` requires per-call approval")
        }
        AskReason::BrowserChromeAccess { action } => {
            format!("browser Chrome access `{action}` requires approval")
        }
        AskReason::BrowserDownloadAction { action } => {
            format!("browser download action `{action}` requires approval")
        }
        AskReason::MacControlAction { action } => {
            format!("macOS control action `{action}` requires approval")
        }
        AskReason::MacControlDangerousAction { action } => {
            format!("dangerous macOS control action `{action}` requires approval")
        }
        AskReason::PlanModeAsk => "Plan Mode requires explicit approval".to_string(),
        AskReason::CronDelete => "cron deletion requires approval".to_string(),
    }
}

fn allow_call(
    api: String,
    line: usize,
    tool_name: Option<String>,
    label: Option<String>,
    args: Option<Value>,
    reason: Option<String>,
) -> WorkflowPermissionPreviewCall {
    WorkflowPermissionPreviewCall {
        api,
        line,
        tool_name,
        decision: "allow".to_string(),
        strict: false,
        dynamic: false,
        reason,
        label,
        args,
    }
}

fn dynamic_call(api: String, line: usize, reason: &str) -> WorkflowPermissionPreviewCall {
    WorkflowPermissionPreviewCall {
        api,
        line,
        tool_name: None,
        decision: "dynamic".to_string(),
        strict: false,
        dynamic: true,
        reason: Some(reason.to_string()),
        label: None,
        args: None,
    }
}

fn normalize_tool_args(tool_name: &str, mut args: Value) -> Value {
    if tool_name == tools::TOOL_EXEC {
        if let Value::Object(map) = &mut args {
            if !map.contains_key("command") {
                if let Some(cmd) = map.get("cmd").cloned() {
                    map.insert("command".to_string(), cmd);
                }
            }
        }
    }
    args
}

fn validation_exec_args(args: &Value) -> Result<Vec<Value>> {
    let default_cwd = optional_string(args, "cwd");
    let default_timeout = args.get("timeout").and_then(Value::as_u64);
    let raw = args
        .get("commands")
        .or_else(|| args.get("command"))
        .ok_or_else(|| anyhow!("workflow.validate requires static commands"))?;
    let mut out = Vec::new();
    match raw {
        Value::String(command) => out.push(validation_exec_arg(
            command,
            default_cwd.as_deref(),
            default_timeout,
        )?),
        Value::Array(commands) => {
            for command in commands {
                match command {
                    Value::String(command) => out.push(validation_exec_arg(
                        command,
                        default_cwd.as_deref(),
                        default_timeout,
                    )?),
                    Value::Object(map) => {
                        let command = map
                            .get("command")
                            .or_else(|| map.get("cmd"))
                            .and_then(Value::as_str)
                            .ok_or_else(|| {
                                anyhow!("workflow.validate command object requires command")
                            })?;
                        let cwd = map
                            .get("cwd")
                            .and_then(Value::as_str)
                            .or(default_cwd.as_deref());
                        let timeout = map
                            .get("timeout")
                            .and_then(Value::as_u64)
                            .or(default_timeout);
                        out.push(validation_exec_arg(command, cwd, timeout)?);
                    }
                    _ => {
                        return Err(anyhow!(
                            "workflow.validate commands must be static strings or objects"
                        ))
                    }
                }
            }
        }
        _ => {
            return Err(anyhow!(
                "workflow.validate commands must be a static string or array"
            ))
        }
    }
    if out.is_empty() {
        return Err(anyhow!("workflow.validate requires at least one command"));
    }
    Ok(out)
}

fn validation_exec_arg(command: &str, cwd: Option<&str>, timeout: Option<u64>) -> Result<Value> {
    let command = command.trim();
    if command.is_empty() {
        return Err(anyhow!("workflow.validate command must not be empty"));
    }
    let mut map = serde_json::Map::new();
    map.insert("command".to_string(), Value::String(command.to_string()));
    if let Some(cwd) = cwd.map(str::trim).filter(|s| !s.is_empty()) {
        map.insert("cwd".to_string(), Value::String(cwd.to_string()));
    }
    if let Some(timeout) = timeout {
        map.insert("timeout".to_string(), Value::Number(timeout.into()));
    }
    Ok(Value::Object(map))
}

fn optional_string(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
}

#[derive(Debug)]
struct RawWorkflowCall {
    api: String,
    line: usize,
    args: Option<Value>,
}

fn collect_raw_calls(script: &str) -> Vec<RawWorkflowCall> {
    const PATTERNS: &[(&str, &str)] = &[
        ("workflow.task.create", "workflow.task.create("),
        ("workflow.task.update", "workflow.task.update("),
        ("workflow.fileSearch", "workflow.fileSearch("),
        ("workflow.tool", "workflow.tool("),
        ("workflow.read", "workflow.read("),
        ("workflow.grep", "workflow.grep("),
        ("workflow.validate", "workflow.validate("),
        ("workflow.review", "workflow.review("),
        ("workflow.verify", "workflow.verify("),
        ("workflow.spawnAgent", "workflow.spawnAgent("),
        ("workflow.askUser", "workflow.askUser("),
        ("workflow.diff", "workflow.diff("),
        ("workflow.trace", "workflow.trace("),
    ];

    let mut matches = Vec::new();
    for (api, needle) in PATTERNS {
        for (idx, _) in script.match_indices(needle) {
            matches.push((idx, *api, *needle));
        }
    }
    matches.sort_by_key(|(idx, _, _)| *idx);

    matches
        .into_iter()
        .map(|(idx, api, needle)| {
            let arg_start = idx + needle.len();
            let line = line_number(script, idx);
            let args = first_static_object_arg(script, arg_start).ok();
            RawWorkflowCall {
                api: api.to_string(),
                line,
                args,
            }
        })
        .collect()
}

fn first_static_object_arg(script: &str, arg_start: usize) -> Result<Value> {
    let open = script[arg_start..]
        .char_indices()
        .find(|(_, ch)| !ch.is_whitespace())
        .map(|(offset, ch)| (arg_start + offset, ch))
        .ok_or_else(|| anyhow!("empty call"))?;
    if open.1 != '{' {
        return Err(anyhow!("first argument is not an object literal"));
    }
    let object = extract_balanced(script, open.0, '{', '}')
        .ok_or_else(|| anyhow!("unterminated object literal"))?;
    JsLiteralParser::new(object).parse()
}

fn line_number(script: &str, idx: usize) -> usize {
    script[..idx].bytes().filter(|b| *b == b'\n').count() + 1
}

fn extract_balanced(script: &str, open_idx: usize, open: char, close: char) -> Option<&str> {
    let mut depth = 0usize;
    let mut state = ScanState::Code;
    let mut iter = script[open_idx..].char_indices().peekable();
    while let Some((rel, ch)) = iter.next() {
        match state {
            ScanState::Code => {
                if ch == '/' {
                    if let Some((_, next)) = iter.peek().copied() {
                        if next == '/' {
                            iter.next();
                            state = ScanState::LineComment;
                            continue;
                        }
                        if next == '*' {
                            iter.next();
                            state = ScanState::BlockComment;
                            continue;
                        }
                    }
                }
                if matches!(ch, '"' | '\'' | '`') {
                    state = ScanState::String(ch);
                    continue;
                }
                if ch == open {
                    depth += 1;
                } else if ch == close {
                    depth = depth.checked_sub(1)?;
                    if depth == 0 {
                        let end = open_idx + rel + ch.len_utf8();
                        return Some(&script[open_idx..end]);
                    }
                }
            }
            ScanState::String(quote) => {
                if ch == '\\' {
                    iter.next();
                } else if ch == quote {
                    state = ScanState::Code;
                }
            }
            ScanState::LineComment => {
                if ch == '\n' {
                    state = ScanState::Code;
                }
            }
            ScanState::BlockComment => {
                if ch == '*' {
                    if let Some((_, next)) = iter.peek().copied() {
                        if next == '/' {
                            iter.next();
                            state = ScanState::Code;
                        }
                    }
                }
            }
        }
    }
    None
}

#[derive(Debug, Clone, Copy)]
enum ScanState {
    Code,
    String(char),
    LineComment,
    BlockComment,
}

struct JsLiteralParser<'a> {
    chars: Vec<char>,
    pos: usize,
    source: &'a str,
}

impl<'a> JsLiteralParser<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            chars: source.chars().collect(),
            pos: 0,
            source,
        }
    }

    fn parse(mut self) -> Result<Value> {
        let value = self.parse_value()?;
        self.skip_ws();
        if self.pos != self.chars.len() {
            return Err(anyhow!("unexpected trailing object literal content"));
        }
        Ok(value)
    }

    fn parse_value(&mut self) -> Result<Value> {
        self.skip_ws();
        match self.peek() {
            Some('{') => self.parse_object(),
            Some('[') => self.parse_array(),
            Some('"') | Some('\'') => self.parse_string().map(Value::String),
            Some('-') | Some('0'..='9') => self.parse_number(),
            Some(ch) if is_identifier_start(ch) => self.parse_identifier_value(),
            Some(ch) => Err(anyhow!("unsupported literal value starting with `{ch}`")),
            None => Err(anyhow!("unexpected end of literal")),
        }
    }

    fn parse_object(&mut self) -> Result<Value> {
        self.expect('{')?;
        let mut map = serde_json::Map::new();
        loop {
            self.skip_ws();
            if self.consume('}') {
                break;
            }
            let key = self.parse_key()?;
            self.skip_ws();
            self.expect(':')?;
            let value = self.parse_value()?;
            map.insert(key, value);
            self.skip_ws();
            if self.consume('}') {
                break;
            }
            self.expect(',')?;
        }
        Ok(Value::Object(map))
    }

    fn parse_array(&mut self) -> Result<Value> {
        self.expect('[')?;
        let mut values = Vec::new();
        loop {
            self.skip_ws();
            if self.consume(']') {
                break;
            }
            values.push(self.parse_value()?);
            self.skip_ws();
            if self.consume(']') {
                break;
            }
            self.expect(',')?;
        }
        Ok(Value::Array(values))
    }

    fn parse_key(&mut self) -> Result<String> {
        self.skip_ws();
        match self.peek() {
            Some('"') | Some('\'') => self.parse_string(),
            Some(ch) if is_identifier_start(ch) => self.parse_identifier(),
            Some(ch) => Err(anyhow!("unsupported object key starting with `{ch}`")),
            None => Err(anyhow!("unexpected end of object key")),
        }
    }

    fn parse_string(&mut self) -> Result<String> {
        let quote = self.next().ok_or_else(|| anyhow!("expected string"))?;
        let mut out = String::new();
        while let Some(ch) = self.next() {
            if ch == quote {
                return Ok(out);
            }
            if ch == '\\' {
                let escaped = self
                    .next()
                    .ok_or_else(|| anyhow!("unterminated string escape"))?;
                match escaped {
                    '"' | '\'' | '\\' | '/' => out.push(escaped),
                    'n' => out.push('\n'),
                    'r' => out.push('\r'),
                    't' => out.push('\t'),
                    'b' => out.push('\u{0008}'),
                    'f' => out.push('\u{000c}'),
                    'u' => {
                        let mut hex = String::new();
                        for _ in 0..4 {
                            let h = self
                                .next()
                                .ok_or_else(|| anyhow!("unterminated unicode escape"))?;
                            hex.push(h);
                        }
                        let code = u16::from_str_radix(&hex, 16)
                            .map_err(|_| anyhow!("invalid unicode escape"))?;
                        let ch = char::from_u32(code as u32)
                            .ok_or_else(|| anyhow!("invalid unicode scalar"))?;
                        out.push(ch);
                    }
                    other => return Err(anyhow!("unsupported string escape `\\{other}`")),
                }
            } else {
                out.push(ch);
            }
        }
        Err(anyhow!("unterminated string literal"))
    }

    fn parse_number(&mut self) -> Result<Value> {
        let start = self.pos;
        if self.peek() == Some('-') {
            self.pos += 1;
        }
        while matches!(self.peek(), Some('0'..='9')) {
            self.pos += 1;
        }
        if self.peek() == Some('.') {
            self.pos += 1;
            while matches!(self.peek(), Some('0'..='9')) {
                self.pos += 1;
            }
        }
        if matches!(self.peek(), Some('e' | 'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some('+' | '-')) {
                self.pos += 1;
            }
            while matches!(self.peek(), Some('0'..='9')) {
                self.pos += 1;
            }
        }
        let raw: String = self.chars[start..self.pos].iter().collect();
        if raw.contains(['.', 'e', 'E']) {
            let n = raw.parse::<f64>()?;
            let number = Number::from_f64(n).ok_or_else(|| anyhow!("invalid number"))?;
            Ok(Value::Number(number))
        } else if raw.starts_with('-') {
            Ok(Value::Number(raw.parse::<i64>()?.into()))
        } else {
            Ok(Value::Number(raw.parse::<u64>()?.into()))
        }
    }

    fn parse_identifier_value(&mut self) -> Result<Value> {
        let ident = self.parse_identifier()?;
        match ident.as_str() {
            "true" => Ok(Value::Bool(true)),
            "false" => Ok(Value::Bool(false)),
            "null" | "undefined" => Ok(Value::Null),
            _ => Err(anyhow!(
                "dynamic identifier `{ident}` is not previewable as a static literal"
            )),
        }
    }

    fn parse_identifier(&mut self) -> Result<String> {
        let start = self.pos;
        let Some(ch) = self.peek() else {
            return Err(anyhow!("expected identifier"));
        };
        if !is_identifier_start(ch) {
            return Err(anyhow!("expected identifier"));
        }
        self.pos += 1;
        while self.peek().is_some_and(is_identifier_continue) {
            self.pos += 1;
        }
        Ok(self.chars[start..self.pos].iter().collect())
    }

    fn skip_ws(&mut self) {
        loop {
            while self.peek().is_some_and(char::is_whitespace) {
                self.pos += 1;
            }
            if self.peek() == Some('/') && self.peek_n(1) == Some('/') {
                self.pos += 2;
                while self.peek().is_some_and(|ch| ch != '\n') {
                    self.pos += 1;
                }
                continue;
            }
            if self.peek() == Some('/') && self.peek_n(1) == Some('*') {
                self.pos += 2;
                while self.pos + 1 < self.chars.len() {
                    if self.peek() == Some('*') && self.peek_n(1) == Some('/') {
                        self.pos += 2;
                        break;
                    }
                    self.pos += 1;
                }
                continue;
            }
            break;
        }
    }

    fn expect(&mut self, ch: char) -> Result<()> {
        self.skip_ws();
        if self.consume(ch) {
            Ok(())
        } else {
            Err(anyhow!("expected `{ch}` in literal `{}`", self.source))
        }
    }

    fn consume(&mut self, ch: char) -> bool {
        if self.peek() == Some(ch) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_n(&self, n: usize) -> Option<char> {
        self.chars.get(self.pos + n).copied()
    }

    fn next(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.pos += 1;
        Some(ch)
    }
}

fn is_identifier_start(ch: char) -> bool {
    ch == '_' || ch == '$' || ch.is_ascii_alphabetic()
}

fn is_identifier_continue(ch: char) -> bool {
    is_identifier_start(ch) || ch.is_ascii_digit()
}
