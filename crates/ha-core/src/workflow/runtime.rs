use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context as _, Result};
use rquickjs::function::Opt;
use rquickjs::prelude::{Func, MutFn};
use rquickjs::{
    CatchResultExt, Context, Ctx, Exception, Function, Object, Runtime, Value as JsValue,
};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::runtime::Handle as TokioHandle;

use crate::plan::{check_workflow_script_draft, ScriptGateOptions};
use crate::session::{SessionDB, Task, TaskStatus};
use crate::tools::{self, ToolExecContext};

use super::types::{
    UpsertWorkflowOpInput, WorkflowEffectClass, WorkflowOpState, WorkflowRunSnapshot,
    WorkflowRunState,
};

const DEFAULT_SCRIPT_TIMEOUT_SECS: u64 = 30;
const MAX_SCRIPT_TIMEOUT_SECS: u64 = 300;
const SCRIPT_MEMORY_LIMIT_BYTES: usize = 64 * 1024 * 1024;
const SCRIPT_STACK_LIMIT_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRuntimeResult {
    pub snapshot: WorkflowRunSnapshot,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRecoveryReport {
    pub owner: String,
    pub attempted: usize,
    pub recovered: usize,
    pub blocked: usize,
    pub failed: usize,
    pub skipped: usize,
    pub errors: Vec<String>,
}

pub async fn recover_pending_workflow_runs(
    db: Arc<SessionDB>,
    owner: impl Into<String>,
) -> Result<WorkflowRecoveryReport> {
    let owner = owner.into();
    let mut report = WorkflowRecoveryReport {
        owner: owner.clone(),
        ..Default::default()
    };
    let runs = db
        .list_recoverable_workflow_runs()
        .context("list recoverable workflow runs")?;

    for run in runs {
        let Some(claimed) = db
            .claim_workflow_run_for_recovery(&run.id, &owner)
            .with_context(|| format!("claim workflow run {} for recovery", run.id))?
        else {
            report.skipped += 1;
            continue;
        };
        report.attempted += 1;

        match run_workflow_script_async(db.clone(), &claimed.id).await {
            Ok(result) => match result.snapshot.run.state {
                WorkflowRunState::Completed => report.recovered += 1,
                WorkflowRunState::Blocked => report.blocked += 1,
                WorkflowRunState::Failed => report.failed += 1,
                _ => {}
            },
            Err(err) => {
                let state = db
                    .get_workflow_run(&claimed.id)
                    .ok()
                    .flatten()
                    .map(|run| run.state);
                match state {
                    Some(WorkflowRunState::Blocked) => report.blocked += 1,
                    Some(WorkflowRunState::Failed) => report.failed += 1,
                    _ => report.failed += 1,
                }
                report.errors.push(format!("{}: {err:#}", claimed.id));
            }
        }
    }

    Ok(report)
}

pub fn spawn_startup_recovery_if_primary() {
    if !crate::runtime_lock::is_primary() {
        return;
    }
    let Some(db) = crate::get_session_db() else {
        return;
    };
    let owner = format!("startup:pid:{}", std::process::id());
    tokio::spawn(async move {
        match recover_pending_workflow_runs(db.clone(), owner).await {
            Ok(report) => {
                if report.attempted > 0 || report.skipped > 0 || !report.errors.is_empty() {
                    crate::app_info!(
                        "workflow",
                        "startup_recovery",
                        "owner={} attempted={} recovered={} blocked={} failed={} skipped={} errors={}",
                        report.owner,
                        report.attempted,
                        report.recovered,
                        report.blocked,
                        report.failed,
                        report.skipped,
                        report.errors.len()
                    );
                }
            }
            Err(err) => {
                crate::app_warn!(
                    "workflow",
                    "startup_recovery",
                    "workflow startup recovery failed: {err:#}"
                );
            }
        }
    });
}

pub fn run_workflow_script(db: Arc<SessionDB>, run_id: &str) -> Result<WorkflowRuntimeResult> {
    if TokioHandle::try_current().is_ok() {
        return Err(anyhow!(
            "run_workflow_script was called from an async runtime; use run_workflow_script_async"
        ));
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("create workflow runtime executor")?;
    runtime.block_on(run_workflow_script_async(db, run_id))
}

pub async fn run_workflow_script_async(
    db: Arc<SessionDB>,
    run_id: &str,
) -> Result<WorkflowRuntimeResult> {
    let run = db
        .get_workflow_run(run_id)?
        .ok_or_else(|| anyhow!("workflow run {} not found", run_id))?;

    if run.state == WorkflowRunState::Completed {
        return Ok(WorkflowRuntimeResult {
            snapshot: db
                .workflow_run_snapshot(run_id, 500)?
                .ok_or_else(|| anyhow!("workflow run {} not found", run_id))?,
            output: None,
        });
    }
    if matches!(
        run.state,
        WorkflowRunState::Failed | WorkflowRunState::Cancelled | WorkflowRunState::Blocked
    ) {
        return Err(anyhow!(
            "workflow run {} is terminal ({}); refusing to execute script",
            run_id,
            run.state.as_str()
        ));
    }
    if run.state == WorkflowRunState::Paused {
        return Err(anyhow!("workflow run {} is paused", run_id));
    }

    let gate = check_workflow_script_draft(&run.script_source, ScriptGateOptions::default());
    if !gate.passed() {
        return Err(anyhow!(gate.render_feedback("Workflow Script Gate")));
    }

    if run.state != WorkflowRunState::Running {
        db.transition_workflow_run(run_id, WorkflowRunState::Running, Some("runtime_start"))?;
    }

    let session_context = workflow_session_context(&db, &run.session_id);
    let tokio_handle = TokioHandle::current();
    let db_for_script = db.clone();
    let run_for_script = run.clone();
    let output = match tokio::task::spawn_blocking(move || {
        execute_script(db_for_script, run_for_script, session_context, tokio_handle)
    })
    .await
    .context("workflow runtime worker panicked or was cancelled")?
    {
        Ok(output) => output,
        Err(err) => {
            let _ =
                db.transition_workflow_run(run_id, WorkflowRunState::Failed, Some("runtime_error"));
            return Err(err);
        }
    };

    let snapshot = db
        .workflow_run_snapshot(run_id, 500)?
        .ok_or_else(|| anyhow!("workflow run {} not found", run_id))?;
    Ok(WorkflowRuntimeResult {
        snapshot,
        output: Some(output),
    })
}

fn execute_script(
    db: Arc<SessionDB>,
    run: super::types::WorkflowRun,
    session_context: WorkflowSessionContext,
    tokio_handle: TokioHandle,
) -> Result<Value> {
    let runtime = Runtime::new().context("create QuickJS runtime")?;
    runtime.set_memory_limit(SCRIPT_MEMORY_LIMIT_BYTES);
    runtime.set_max_stack_size(SCRIPT_STACK_LIMIT_BYTES);

    let timeout = script_timeout(&run);
    let started_at = Instant::now();
    runtime.set_interrupt_handler(Some(Box::new(move || started_at.elapsed() >= timeout)));

    let ctx = Context::full(&runtime).context("create QuickJS context")?;
    ctx.with(|ctx| -> Result<Value> {
        let host = Rc::new(RefCell::new(WorkflowRuntimeHost::new(
            db.clone(),
            run.id.clone(),
            run.session_id.clone(),
            session_context.clone(),
            tokio_handle.clone(),
        )));
        let workflow = build_workflow_object(ctx.clone(), host.clone())?;
        ctx.globals()
            .set("workflow", workflow.clone())
            .context("install workflow global")?;
        install_runtime_guards(&ctx)?;

        let script = prepare_script_for_eval(&run.script_source);
        ctx.eval::<(), _>(script)
            .catch(&ctx)
            .map_err(|err| anyhow!("workflow script load failed: {}", err))?;

        let main: Function = ctx
            .globals()
            .get("__hopeWorkflowMain")
            .context("workflow script must export default function main(workflow)")?;
        let raw = main
            .call::<_, JsValue>((workflow,))
            .catch(&ctx)
            .map_err(|err| anyhow!("workflow script failed: {}", err))?;
        let _returned = finish_maybe_promise(ctx.clone(), raw)
            .map_err(|err| anyhow!("workflow script promise failed: {}", err))?;

        let finished = host
            .borrow()
            .finished_output
            .clone()
            .ok_or_else(|| anyhow!("workflow script exited without workflow.finish(result)"))?;
        Ok(finished)
    })
}

fn build_workflow_object<'js>(
    ctx: Ctx<'js>,
    host: Rc<RefCell<WorkflowRuntimeHost>>,
) -> rquickjs::Result<Object<'js>> {
    let workflow = Object::new(ctx.clone())?;
    let task = Object::new(ctx.clone())?;

    let create_host = host.clone();
    task.set(
        "create",
        Func::from(MutFn::from(
            move |ctx: Ctx<'js>, args: JsValue<'js>| -> rquickjs::Result<JsValue<'js>> {
                host_call(&ctx, &create_host, args, WorkflowRuntimeHost::task_create)
            },
        )),
    )?;

    let update_host = host.clone();
    task.set(
        "update",
        Func::from(MutFn::from(
            move |ctx: Ctx<'js>, args: JsValue<'js>| -> rquickjs::Result<JsValue<'js>> {
                host_call(&ctx, &update_host, args, WorkflowRuntimeHost::task_update)
            },
        )),
    )?;
    workflow.set("task", task)?;

    let file_search_host = host.clone();
    workflow.set(
        "fileSearch",
        Func::from(MutFn::from(
            move |ctx: Ctx<'js>, args: JsValue<'js>| -> rquickjs::Result<JsValue<'js>> {
                host_call(
                    &ctx,
                    &file_search_host,
                    args,
                    WorkflowRuntimeHost::file_search,
                )
            },
        )),
    )?;

    let tool_host = host.clone();
    workflow.set(
        "tool",
        Func::from(MutFn::from(
            move |ctx: Ctx<'js>, args: JsValue<'js>| -> rquickjs::Result<JsValue<'js>> {
                host_call(&ctx, &tool_host, args, WorkflowRuntimeHost::tool)
            },
        )),
    )?;

    let spawn_agent_host = host.clone();
    workflow.set(
        "spawnAgent",
        Func::from(MutFn::from(
            move |ctx: Ctx<'js>, args: JsValue<'js>| -> rquickjs::Result<JsValue<'js>> {
                host_call(
                    &ctx,
                    &spawn_agent_host,
                    args,
                    WorkflowRuntimeHost::spawn_agent,
                )
            },
        )),
    )?;

    let wait_all_host = host.clone();
    workflow.set(
        "waitAll",
        Func::from(MutFn::from(
            move |ctx: Ctx<'js>,
                  handles: JsValue<'js>,
                  options: Opt<JsValue<'js>>|
                  -> rquickjs::Result<JsValue<'js>> {
                wait_all_host_call(&ctx, &wait_all_host, handles, options)
            },
        )),
    )?;

    let read_host = host.clone();
    workflow.set(
        "read",
        Func::from(MutFn::from(
            move |ctx: Ctx<'js>, args: JsValue<'js>| -> rquickjs::Result<JsValue<'js>> {
                host_call(&ctx, &read_host, args, WorkflowRuntimeHost::read)
            },
        )),
    )?;

    let grep_host = host.clone();
    workflow.set(
        "grep",
        Func::from(MutFn::from(
            move |ctx: Ctx<'js>, args: JsValue<'js>| -> rquickjs::Result<JsValue<'js>> {
                host_call(&ctx, &grep_host, args, WorkflowRuntimeHost::grep)
            },
        )),
    )?;

    let validate_host = host.clone();
    workflow.set(
        "validate",
        Func::from(MutFn::from(
            move |ctx: Ctx<'js>, args: JsValue<'js>| -> rquickjs::Result<JsValue<'js>> {
                host_call(&ctx, &validate_host, args, WorkflowRuntimeHost::validate)
            },
        )),
    )?;

    let trace_host = host.clone();
    workflow.set(
        "trace",
        Func::from(MutFn::from(
            move |ctx: Ctx<'js>, args: JsValue<'js>| -> rquickjs::Result<JsValue<'js>> {
                host_call(&ctx, &trace_host, args, WorkflowRuntimeHost::trace)
            },
        )),
    )?;

    let finish_host = host.clone();
    workflow.set(
        "finish",
        Func::from(MutFn::from(
            move |ctx: Ctx<'js>, args: JsValue<'js>| -> rquickjs::Result<JsValue<'js>> {
                host_call(&ctx, &finish_host, args, WorkflowRuntimeHost::finish)
            },
        )),
    )?;

    Ok(workflow)
}

fn install_runtime_guards(ctx: &Ctx<'_>) -> Result<()> {
    ctx.eval::<(), _>(
        r#"
        const __HopeNativeDate = Date;
        function __hopeDeterminismError(name) {
          throw new Error(`${name} is disabled in workflow runtime; use a workflow host API deterministic source instead`);
        }
        function HopeWorkflowDate(...args) {
          if (args.length === 0) {
            __hopeDeterminismError("new Date()");
          }
          if (new.target) {
            return Reflect.construct(__HopeNativeDate, args, new.target);
          }
          return __HopeNativeDate(...args);
        }
        Object.setPrototypeOf(HopeWorkflowDate, __HopeNativeDate);
        HopeWorkflowDate.prototype = __HopeNativeDate.prototype;
        HopeWorkflowDate.now = () => __hopeDeterminismError("Date.now()");
        HopeWorkflowDate.parse = __HopeNativeDate.parse;
        HopeWorkflowDate.UTC = __HopeNativeDate.UTC;
        globalThis.Date = HopeWorkflowDate;
        Math.random = () => __hopeDeterminismError("Math.random()");
        "#,
    )
    .catch(ctx)
    .map_err(|err| anyhow!("install workflow runtime guards failed: {}", err))
}

fn host_call<'js>(
    ctx: &Ctx<'js>,
    host: &Rc<RefCell<WorkflowRuntimeHost>>,
    args: JsValue<'js>,
    f: fn(&mut WorkflowRuntimeHost, Value) -> Result<Value>,
) -> rquickjs::Result<JsValue<'js>> {
    let args = js_to_json(ctx, args)?;
    let output = host
        .try_borrow_mut()
        .map_err(|_| Exception::throw_message(ctx, "workflow host API called recursively"))?
        .call(args, f)
        .map_err(|err| js_error(ctx, err))?;
    json_to_js(ctx.clone(), output)
}

fn wait_all_host_call<'js>(
    ctx: &Ctx<'js>,
    host: &Rc<RefCell<WorkflowRuntimeHost>>,
    handles: JsValue<'js>,
    options: Opt<JsValue<'js>>,
) -> rquickjs::Result<JsValue<'js>> {
    let handles = js_to_json(ctx, handles)?;
    let options = options
        .0
        .filter(|value| !value.is_undefined() && !value.is_null())
        .map(|value| js_to_json(ctx, value))
        .transpose()?;
    let args = wait_all_args_from_values(handles, options).map_err(|err| js_error(ctx, err))?;
    let output = host
        .try_borrow_mut()
        .map_err(|_| Exception::throw_message(ctx, "workflow host API called recursively"))?
        .call(args, WorkflowRuntimeHost::wait_all)
        .map_err(|err| js_error(ctx, err))?;
    json_to_js(ctx.clone(), output)
}

fn js_to_json<'js>(ctx: &Ctx<'js>, value: JsValue<'js>) -> rquickjs::Result<Value> {
    rquickjs_serde::from_value_strict(value)
        .map_err(|err| Exception::throw_message(ctx, &format!("invalid workflow host args: {err}")))
}

fn json_to_js<'js>(ctx: Ctx<'js>, value: Value) -> rquickjs::Result<JsValue<'js>> {
    rquickjs_serde::to_value(ctx.clone(), value)
        .map_err(|err| Exception::throw_message(&ctx, &format!("serialize workflow result: {err}")))
}

fn js_error<'js>(ctx: &Ctx<'js>, err: anyhow::Error) -> rquickjs::Error {
    Exception::throw_message(ctx, &format!("{err:#}"))
}

fn finish_maybe_promise<'js>(
    ctx: Ctx<'js>,
    value: JsValue<'js>,
) -> rquickjs::CaughtResult<'js, JsValue<'js>> {
    if value.is_promise() {
        let promise = value.into_promise().expect("checked promise");
        promise.finish::<JsValue>().catch(&ctx)
    } else {
        Ok(value)
    }
}

fn prepare_script_for_eval(script: &str) -> String {
    let trimmed = script.trim();
    let prepared = if trimmed.contains("export default") {
        trimmed.replacen("export default", "globalThis.__hopeWorkflowMain =", 1)
    } else {
        let mut s = trimmed.to_string();
        if !s.contains("__hopeWorkflowMain") && s.contains("function main") {
            s.push_str("\nglobalThis.__hopeWorkflowMain = main;");
        }
        s
    };
    format!("\"use strict\";\n{prepared}")
}

fn script_timeout(run: &super::types::WorkflowRun) -> Duration {
    let secs = run
        .budget
        .get("maxScriptSecs")
        .or_else(|| run.budget.get("max_script_secs"))
        .or_else(|| run.budget.get("maxRuntimeSecs"))
        .or_else(|| run.budget.get("max_runtime_secs"))
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_SCRIPT_TIMEOUT_SECS)
        .clamp(1, MAX_SCRIPT_TIMEOUT_SECS);
    Duration::from_secs(secs)
}

struct WorkflowRuntimeHost {
    db: Arc<SessionDB>,
    run_id: String,
    session_id: String,
    session_context: WorkflowSessionContext,
    tokio_handle: TokioHandle,
    next_op_index: usize,
    finished_output: Option<Value>,
}

impl WorkflowRuntimeHost {
    fn new(
        db: Arc<SessionDB>,
        run_id: String,
        session_id: String,
        session_context: WorkflowSessionContext,
        tokio_handle: TokioHandle,
    ) -> Self {
        Self {
            db,
            run_id,
            session_id,
            session_context,
            tokio_handle,
            next_op_index: 0,
            finished_output: None,
        }
    }

    fn call(
        &mut self,
        args: Value,
        f: fn(&mut WorkflowRuntimeHost, Value) -> Result<Value>,
    ) -> Result<Value> {
        f(self, args)
    }

    fn task_create(&mut self, args: Value) -> Result<Value> {
        let title = required_string(&args, "title")?;
        let label = optional_string(&args, "label");
        let input = compact_input(args);
        self.execute_op(
            "task.create",
            WorkflowEffectClass::Idempotent,
            input,
            |host| {
                let task = host
                    .db
                    .create_task(&host.session_id, &title, None)
                    .context("create workflow task")?;
                let tasks = host.db.list_tasks(&host.session_id).unwrap_or_default();
                crate::session::emit_task_snapshot(&host.session_id, &tasks);
                crate::hooks::fire_task_created(&host.session_id, &title, None, "");
                Ok(task_handle(&task, label.as_deref()))
            },
        )
    }

    fn task_update(&mut self, args: Value) -> Result<Value> {
        let id = task_id_from_args(&args)?;
        let status = optional_string(&args, "status")
            .map(|value| {
                TaskStatus::from_str(&value)
                    .ok_or_else(|| anyhow!("invalid task status '{}'", value))
            })
            .transpose()?;
        let content = optional_string(&args, "title").or_else(|| optional_string(&args, "content"));
        let active_form = optional_string(&args, "activeForm");
        if status.is_none() && content.is_none() && active_form.is_none() {
            return Err(anyhow!(
                "workflow.task.update requires status, title/content, or activeForm"
            ));
        }

        let input = compact_input(args);
        self.execute_op(
            "task.update",
            WorkflowEffectClass::Idempotent,
            input,
            |host| {
                let current = host.db.list_tasks(&host.session_id)?;
                if !current.iter().any(|task| task.id == id) {
                    return Err(anyhow!(
                        "task {} does not belong to workflow session {}",
                        id,
                        host.session_id
                    ));
                }
                let updated =
                    host.db
                        .update_task(id, status, content.as_deref(), active_form.as_deref())?;
                let tasks = host.db.list_tasks(&host.session_id).unwrap_or_default();
                crate::session::emit_task_snapshot(&host.session_id, &tasks);
                if status == Some(TaskStatus::Completed) {
                    crate::hooks::fire_task_completed(&host.session_id, id, &updated.content);
                }
                Ok(task_handle(&updated, None))
            },
        )
    }

    fn file_search(&mut self, args: Value) -> Result<Value> {
        let query = required_string(&args, "query")?;
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .map(|n| n as usize);
        let root = optional_string(&args, "root")
            .or_else(|| self.session_context.working_dir.clone())
            .ok_or_else(|| anyhow!("workflow.fileSearch requires a session working directory"))?;
        let input = json!({
            "query": query.clone(),
            "limit": limit,
            "root": root.clone(),
            "label": optional_string(&args, "label"),
        });
        self.execute_op("fileSearch", WorkflowEffectClass::Pure, input, |_host| {
            let response = crate::filesystem::search_files(&root, &query, limit)
                .context("workflow.fileSearch failed")?;
            serde_json::to_value(response).context("serialize fileSearch response")
        })
    }

    fn tool(&mut self, args: Value) -> Result<Value> {
        let name = required_string(&args, "name")?;
        let tool_args = args.get("args").cloned().unwrap_or_else(|| json!({}));
        let label = optional_string(&args, "label");
        let effect_class = tool_effect_class(&name);
        let op_type = format!("tool:{name}");
        let input = json!({
            "name": name.clone(),
            "args": tool_args.clone(),
            "label": label,
        });
        self.execute_op(&op_type, effect_class, input, |host| {
            host.dispatch_tool(&name, &tool_args).map(Value::String)
        })
    }

    fn spawn_agent(&mut self, args: Value) -> Result<Value> {
        let tool_args = spawn_agent_tool_args(&args)?;
        let label = optional_string(&args, "label");
        let task = optional_string(&args, "task");
        let input = json!({
            "args": tool_args.clone(),
            "label": label.clone(),
        });
        let child_handle = uuid::Uuid::new_v4().to_string();
        self.execute_op_with_child_handle(
            "spawnAgent",
            WorkflowEffectClass::NonIdempotent,
            input,
            child_handle,
            |host, child_handle| {
                host.recover_spawn_agent_child(child_handle, label.as_deref(), task.as_deref())
            },
            |host, child_handle| {
                let mut dispatch_args = tool_args.clone();
                inject_workflow_preallocated_run_id(&mut dispatch_args, child_handle)?;
                let output = host.dispatch_tool(tools::TOOL_SUBAGENT, &dispatch_args)?;
                let parsed = parse_tool_json_output(&output, "workflow.spawnAgent")?;
                let run_id = parsed
                    .get("run_id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .ok_or_else(|| anyhow!("workflow.spawnAgent subagent output missing run_id"))?;
                if run_id != child_handle {
                    return Err(anyhow!(
                        "workflow.spawnAgent returned run_id {} but expected preallocated child handle {}",
                        run_id,
                        child_handle
                    ));
                }
                Ok(subagent_handle(
                    &run_id,
                    label.as_deref(),
                    task.as_deref(),
                    &parsed,
                ))
            },
        )
    }

    fn recover_spawn_agent_child(
        &self,
        child_handle: &str,
        label: Option<&str>,
        task: Option<&str>,
    ) -> Result<Option<Value>> {
        let Some(run) = self.db.get_subagent_run(child_handle)? else {
            return Ok(None);
        };
        Ok(Some(subagent_run_handle(&run, label, task)))
    }

    fn wait_all(&mut self, args: Value) -> Result<Value> {
        let tool_args = wait_all_tool_args(&args)?;
        let input = compact_input(args);
        self.execute_op("waitAll", WorkflowEffectClass::Pure, input, |host| {
            let output = host.dispatch_tool(tools::TOOL_SUBAGENT, &tool_args)?;
            let mut parsed = parse_tool_json_output(&output, "workflow.waitAll")?;
            normalize_wait_all_response(&mut parsed);
            Ok(parsed)
        })
    }

    fn read(&mut self, args: Value) -> Result<Value> {
        let tool_args = args.clone();
        self.execute_op("read", WorkflowEffectClass::Pure, args, |host| {
            host.dispatch_tool(tools::TOOL_READ, &tool_args)
                .map(Value::String)
        })
    }

    fn grep(&mut self, args: Value) -> Result<Value> {
        let tool_args = args.clone();
        self.execute_op("grep", WorkflowEffectClass::Pure, args, |host| {
            host.dispatch_tool(tools::TOOL_GREP, &tool_args)
                .map(Value::String)
        })
    }

    fn validate(&mut self, args: Value) -> Result<Value> {
        let commands = validation_commands_from_args(&args)?;
        let reason = optional_string(&args, "reason");
        let input = compact_input(args);
        self.execute_op(
            "validate",
            WorkflowEffectClass::NonIdempotent,
            input,
            |host| {
                let mut results = Vec::with_capacity(commands.len());
                for command in commands {
                    let exec_args = command.exec_args();
                    let output = host.dispatch_validation_exec(&command.command, &exec_args)?;
                    let exit_code = validation_exit_code(&output);
                    results.push(json!({
                        "command": command.command,
                        "cwd": command.cwd,
                        "timeout": command.timeout,
                        "ok": exit_code == 0,
                        "exitCode": exit_code,
                        "output": output,
                    }));
                }
                let failed = results
                    .iter()
                    .filter(|result| !result.get("ok").and_then(Value::as_bool).unwrap_or(false))
                    .count();
                let ok = failed == 0;
                let summary = if ok {
                    format!("{} validation command(s) passed", results.len())
                } else {
                    format!("{failed}/{} validation command(s) failed", results.len())
                };
                Ok(json!({
                    "ok": ok,
                    "summary": summary,
                    "reason": reason,
                    "results": results,
                }))
            },
        )
    }

    fn trace(&mut self, args: Value) -> Result<Value> {
        let label = optional_string(&args, "label");
        let payload = args.get("payload").cloned().unwrap_or(Value::Null);
        let input = compact_input(args);
        self.execute_op("trace", WorkflowEffectClass::Pure, input, |host| {
            let event = host.db.append_workflow_event(
                &host.run_id,
                "trace",
                json!({
                    "label": label,
                    "payload": payload,
                }),
            )?;
            Ok(json!({ "eventSeq": event.seq }))
        })
    }

    fn finish(&mut self, args: Value) -> Result<Value> {
        let output_arg = args.clone();
        let input = compact_input(args);
        let output = self.execute_op("finish", WorkflowEffectClass::Pure, input, |_host| {
            Ok(output_arg)
        })?;
        self.finished_output = Some(output.clone());
        self.db.transition_workflow_run(
            &self.run_id,
            WorkflowRunState::Completed,
            Some("workflow_finish"),
        )?;
        Ok(output)
    }

    fn execute_op<F>(
        &mut self,
        op_type: &str,
        effect_class: WorkflowEffectClass,
        input: Value,
        f: F,
    ) -> Result<Value>
    where
        F: FnOnce(&mut WorkflowRuntimeHost) -> Result<Value>,
    {
        let op_key = self.next_op_key(op_type);
        let existing = self.db.get_workflow_op(&self.run_id, &op_key)?;
        let op = self.db.upsert_workflow_op_started(UpsertWorkflowOpInput {
            run_id: self.run_id.clone(),
            op_key: op_key.clone(),
            op_type: op_type.to_string(),
            effect_class,
            input,
            child_handle: None,
        })?;

        match op.state {
            WorkflowOpState::Completed => return Ok(op.output.unwrap_or(Value::Null)),
            WorkflowOpState::Failed => {
                return Err(anyhow!(
                    "workflow op {} previously failed: {}",
                    op_key,
                    op.error.unwrap_or(Value::Null)
                ));
            }
            WorkflowOpState::Pending | WorkflowOpState::Started => {}
        }
        if existing
            .as_ref()
            .is_some_and(|op| op.state == WorkflowOpState::Started)
        {
            match self.db.started_op_recovery_action(&self.run_id, &op_key)? {
                Some(super::types::StartedOpRecoveryAction::BlockNonIdempotent)
                | Some(super::types::StartedOpRecoveryAction::AttachChildHandle(_)) => {
                    let _ = self
                        .db
                        .block_run_for_started_non_idempotent_op(&self.run_id, &op_key);
                    return Err(anyhow!(
                        "workflow op {} is a previously-started non-idempotent op; run was blocked",
                        op_key
                    ));
                }
                Some(super::types::StartedOpRecoveryAction::RerunPure)
                | Some(super::types::StartedOpRecoveryAction::RecheckIdempotent)
                | None => {}
            }
        }

        let output = match f(self) {
            Ok(output) => output,
            Err(err) => {
                let _ = self.db.fail_workflow_op(
                    &self.run_id,
                    &op_key,
                    json!({ "message": err.to_string() }),
                );
                return Err(err);
            }
        };
        self.db
            .complete_workflow_op(&self.run_id, &op_key, output.clone())?;
        Ok(output)
    }

    fn execute_op_with_child_handle<F, R>(
        &mut self,
        op_type: &str,
        effect_class: WorkflowEffectClass,
        input: Value,
        child_handle: String,
        recover_started_child: R,
        f: F,
    ) -> Result<Value>
    where
        F: FnOnce(&mut WorkflowRuntimeHost, &str) -> Result<Value>,
        R: FnOnce(&mut WorkflowRuntimeHost, &str) -> Result<Option<Value>>,
    {
        let op_key = self.next_op_key(op_type);
        let existing = self.db.get_workflow_op(&self.run_id, &op_key)?;
        let existing_started_without_child = existing
            .as_ref()
            .is_some_and(|op| op.state == WorkflowOpState::Started && op.child_handle.is_none());
        let effective_child_handle = existing
            .as_ref()
            .and_then(|op| op.child_handle.clone())
            .unwrap_or(child_handle);
        let op = self.db.upsert_workflow_op_started(UpsertWorkflowOpInput {
            run_id: self.run_id.clone(),
            op_key: op_key.clone(),
            op_type: op_type.to_string(),
            effect_class,
            input,
            child_handle: if existing_started_without_child {
                None
            } else {
                Some(effective_child_handle.clone())
            },
        })?;

        match op.state {
            WorkflowOpState::Completed => return Ok(op.output.unwrap_or(Value::Null)),
            WorkflowOpState::Failed => {
                return Err(anyhow!(
                    "workflow op {} previously failed: {}",
                    op_key,
                    op.error.unwrap_or(Value::Null)
                ));
            }
            WorkflowOpState::Pending | WorkflowOpState::Started => {}
        }

        if existing
            .as_ref()
            .is_some_and(|op| op.state == WorkflowOpState::Started)
        {
            match self.db.started_op_recovery_action(&self.run_id, &op_key)? {
                Some(super::types::StartedOpRecoveryAction::AttachChildHandle(handle)) => {
                    if let Some(output) = recover_started_child(self, &handle)? {
                        self.db
                            .complete_workflow_op(&self.run_id, &op_key, output.clone())?;
                        return Ok(output);
                    }
                }
                Some(super::types::StartedOpRecoveryAction::BlockNonIdempotent) => {
                    let _ = self
                        .db
                        .block_run_for_started_non_idempotent_op(&self.run_id, &op_key);
                    return Err(anyhow!(
                        "workflow op {} is a previously-started non-idempotent op; run was blocked",
                        op_key
                    ));
                }
                Some(super::types::StartedOpRecoveryAction::RerunPure)
                | Some(super::types::StartedOpRecoveryAction::RecheckIdempotent)
                | None => {}
            }
        }

        let output = match f(self, &effective_child_handle) {
            Ok(output) => output,
            Err(err) => {
                let _ = self.db.fail_workflow_op(
                    &self.run_id,
                    &op_key,
                    json!({ "message": err.to_string() }),
                );
                return Err(err);
            }
        };
        self.db
            .complete_workflow_op(&self.run_id, &op_key, output.clone())?;
        Ok(output)
    }

    fn next_op_key(&mut self, op_type: &str) -> String {
        let idx = self.next_op_index;
        self.next_op_index += 1;
        format!("main/op#{idx}({op_type})")
    }

    fn dispatch_tool(&self, name: &str, args: &Value) -> Result<String> {
        let ctx = self.tool_exec_context();
        let default_path = ctx.default_path().to_string();
        let session_id = self.session_id.clone();
        self.tokio_handle
            .block_on(tools::execute_tool_with_context(name, args, &ctx))
            .with_context(|| {
                format!("workflow.tool({name}) failed (session={session_id}, cwd={default_path})")
            })
    }

    fn dispatch_validation_exec(&self, command: &str, args: &Value) -> Result<String> {
        let mut ctx = self.tool_exec_context();
        ctx.async_tool_policy = crate::agent_config::AsyncToolPolicy::NeverBackground;
        let default_path = ctx.default_cwd();
        let session_id = self.session_id.clone();
        self.tokio_handle
            .block_on(tools::execute_tool_with_context(tools::TOOL_EXEC, args, &ctx))
            .with_context(|| {
                format!(
                    "workflow.validate command failed before completion (session={session_id}, cwd={default_path}, command={command})"
                )
            })
    }

    fn tool_exec_context(&self) -> ToolExecContext {
        ToolExecContext {
            session_id: Some(self.session_id.clone()),
            session_working_dir: self.session_context.working_dir.clone(),
            agent_id: self.session_context.agent_id.clone(),
            session_mode: self.session_context.session_mode,
            project_id: self.session_context.project_id.clone(),
            incognito: self.session_context.incognito,
            ..Default::default()
        }
    }
}

fn task_handle(task: &Task, label: Option<&str>) -> Value {
    json!({
        "id": task.id,
        "sessionId": task.session_id,
        "title": task.content,
        "status": task.status,
        "label": label,
    })
}

pub(crate) fn spawn_agent_tool_args(args: &Value) -> Result<Value> {
    let task = required_string(args, "task")?;
    let mut map = serde_json::Map::new();
    map.insert("action".to_string(), Value::String("spawn".to_string()));
    map.insert("task".to_string(), Value::String(task));
    if let Some(agent_id) =
        optional_string(args, "agent_id").or_else(|| optional_string(args, "agent"))
    {
        map.insert("agent_id".to_string(), Value::String(agent_id));
    }
    if let Some(label) = optional_string(args, "label") {
        map.insert("label".to_string(), Value::String(label));
    }
    if let Some(model) = optional_string(args, "model") {
        map.insert("model".to_string(), Value::String(model));
    }
    if let Some(timeout_secs) = optional_u64_any(args, &["timeout_secs", "timeoutSecs", "timeout"])
    {
        map.insert(
            "timeout_secs".to_string(),
            Value::Number(timeout_secs.into()),
        );
    }
    if let Some(files) = args.get("files") {
        if !files.is_array() {
            return Err(anyhow!("workflow.spawnAgent files must be an array"));
        }
        map.insert("files".to_string(), files.clone());
    }
    Ok(Value::Object(map))
}

fn inject_workflow_preallocated_run_id(args: &mut Value, run_id: &str) -> Result<()> {
    let Value::Object(map) = args else {
        return Err(anyhow!(
            "workflow.spawnAgent internal args must be an object"
        ));
    };
    map.insert(
        tools::subagent::WORKFLOW_PREALLOCATED_RUN_ID_ARG.to_string(),
        Value::String(run_id.to_string()),
    );
    Ok(())
}

fn subagent_handle(run_id: &str, label: Option<&str>, task: Option<&str>, raw: &Value) -> Value {
    json!({
        "kind": "subagent",
        "runId": run_id,
        "run_id": run_id,
        "status": raw.get("status").and_then(Value::as_str).unwrap_or("spawned"),
        "label": label,
        "task": task,
        "message": raw.get("message").cloned().unwrap_or(Value::Null),
    })
}

fn subagent_run_handle(
    run: &crate::subagent::SubagentRun,
    label: Option<&str>,
    task: Option<&str>,
) -> Value {
    json!({
        "kind": "subagent",
        "runId": run.run_id,
        "run_id": run.run_id,
        "status": run.status.as_str(),
        "label": label.map(ToOwned::to_owned).or_else(|| run.label.clone()),
        "task": task.map(ToOwned::to_owned).unwrap_or_else(|| run.task.clone()),
        "message": "attached to existing sub-agent run",
    })
}

fn parse_tool_json_output(output: &str, context: &str) -> Result<Value> {
    serde_json::from_str(output).with_context(|| format!("{context} returned non-JSON output"))
}

fn wait_all_args_from_values(handles: Value, options: Option<Value>) -> Result<Value> {
    let mut map = match handles {
        Value::Object(map)
            if map.contains_key("handles")
                || map.contains_key("runIds")
                || map.contains_key("run_ids") =>
        {
            map
        }
        value => {
            let mut map = serde_json::Map::new();
            map.insert("handles".to_string(), value);
            map
        }
    };

    if let Some(options) = options {
        let Value::Object(options) = options else {
            return Err(anyhow!("workflow.waitAll options must be an object"));
        };
        for (key, value) in options {
            map.insert(key, value);
        }
    }

    Ok(Value::Object(map))
}

pub(crate) fn wait_all_tool_args(args: &Value) -> Result<Value> {
    let handles = args
        .get("handles")
        .or_else(|| args.get("runIds"))
        .or_else(|| args.get("run_ids"))
        .ok_or_else(|| anyhow!("workflow.waitAll requires handles or runIds"))?;
    let run_ids = extract_subagent_run_ids(handles)?;
    if run_ids.is_empty() {
        return Err(anyhow!("workflow.waitAll requires at least one handle"));
    }

    let mut map = serde_json::Map::new();
    map.insert("action".to_string(), Value::String("wait_all".to_string()));
    map.insert("run_ids".to_string(), json!(run_ids));
    if let Some(wait_timeout) = optional_u64_any(args, &["wait_timeout", "waitTimeout", "timeout"])
    {
        map.insert(
            "wait_timeout".to_string(),
            Value::Number(wait_timeout.into()),
        );
    }
    Ok(Value::Object(map))
}

fn extract_subagent_run_ids(value: &Value) -> Result<Vec<String>> {
    match value {
        Value::String(run_id) => Ok(vec![run_id.clone()]),
        Value::Array(items) => {
            let mut run_ids = Vec::with_capacity(items.len());
            for item in items {
                run_ids.extend(extract_subagent_run_ids(item)?);
            }
            Ok(run_ids)
        }
        Value::Object(map) => {
            if let Some(run_id) = map
                .get("runId")
                .or_else(|| map.get("run_id"))
                .and_then(Value::as_str)
            {
                return Ok(vec![run_id.to_string()]);
            }
            if let Some(nested) = map
                .get("handles")
                .or_else(|| map.get("runIds"))
                .or_else(|| map.get("run_ids"))
            {
                return extract_subagent_run_ids(nested);
            }
            Err(anyhow!("workflow.waitAll handle object must include runId"))
        }
        _ => Err(anyhow!(
            "workflow.waitAll handles must be run IDs or subagent handles"
        )),
    }
}

fn normalize_wait_all_response(value: &mut Value) {
    if let Value::Object(map) = value {
        if let Some(all_completed) = map.get("all_completed").cloned() {
            map.entry("allCompleted".to_string())
                .or_insert(all_completed);
        }
        if let Some(Value::Array(runs)) = map.get_mut("runs") {
            for run in runs {
                if let Value::Object(run) = run {
                    if let Some(run_id) = run.get("run_id").cloned() {
                        run.entry("runId".to_string()).or_insert(run_id);
                    }
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
struct ValidationCommand {
    command: String,
    cwd: Option<String>,
    timeout: Option<u64>,
}

impl ValidationCommand {
    fn exec_args(&self) -> Value {
        let mut args = serde_json::Map::new();
        args.insert("command".to_string(), Value::String(self.command.clone()));
        if let Some(cwd) = self.cwd.clone() {
            args.insert("cwd".to_string(), Value::String(cwd));
        }
        if let Some(timeout) = self.timeout {
            args.insert("timeout".to_string(), Value::Number(timeout.into()));
        }
        Value::Object(args)
    }
}

fn validation_commands_from_args(args: &Value) -> Result<Vec<ValidationCommand>> {
    let default_cwd = optional_string(args, "cwd");
    let default_timeout = args.get("timeout").and_then(Value::as_u64);
    let raw_commands = args
        .get("commands")
        .or_else(|| args.get("command"))
        .ok_or_else(|| anyhow!("workflow.validate requires commands"))?;
    let mut commands = Vec::new();
    match raw_commands {
        Value::String(command) => {
            commands.push(ValidationCommand {
                command: normalize_command(command)?,
                cwd: default_cwd,
                timeout: default_timeout,
            });
        }
        Value::Array(items) => {
            for item in items {
                commands.push(validation_command_from_value(
                    item,
                    default_cwd.clone(),
                    default_timeout,
                )?);
            }
        }
        _ => {
            return Err(anyhow!(
                "workflow.validate commands must be a string or array of strings/objects"
            ));
        }
    }
    if commands.is_empty() {
        return Err(anyhow!("workflow.validate requires at least one command"));
    }
    if commands.len() > 8 {
        return Err(anyhow!(
            "workflow.validate supports at most 8 commands per op"
        ));
    }
    Ok(commands)
}

fn validation_command_from_value(
    value: &Value,
    default_cwd: Option<String>,
    default_timeout: Option<u64>,
) -> Result<ValidationCommand> {
    match value {
        Value::String(command) => Ok(ValidationCommand {
            command: normalize_command(command)?,
            cwd: default_cwd,
            timeout: default_timeout,
        }),
        Value::Object(map) => {
            let command = map
                .get("command")
                .or_else(|| map.get("cmd"))
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("workflow.validate command object requires command"))?;
            Ok(ValidationCommand {
                command: normalize_command(command)?,
                cwd: map
                    .get("cwd")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(ToOwned::to_owned)
                    .or(default_cwd),
                timeout: map
                    .get("timeout")
                    .and_then(Value::as_u64)
                    .or(default_timeout),
            })
        }
        _ => Err(anyhow!(
            "workflow.validate command entries must be strings or objects"
        )),
    }
}

fn normalize_command(command: &str) -> Result<String> {
    let command = command.trim();
    if command.is_empty() {
        return Err(anyhow!("workflow.validate command must not be empty"));
    }
    if command.len() > 4096 {
        return Err(anyhow!("workflow.validate command is too long"));
    }
    Ok(command.to_string())
}

pub(crate) fn validation_exit_code(output: &str) -> i64 {
    let trimmed = output.trim();
    if let Some(code) = trimmed
        .strip_prefix("Command completed with exit code ")
        .and_then(|value| value.trim().parse::<i64>().ok())
    {
        return code;
    }
    if let Some(start) = trimmed.rfind("[exit code: ") {
        let after = &trimmed[start + "[exit code: ".len()..];
        if let Some(end) = after.find(']') {
            if let Ok(code) = after[..end].trim().parse::<i64>() {
                return code;
            }
        }
    }
    0
}

fn tool_effect_class(name: &str) -> WorkflowEffectClass {
    match name {
        tools::TOOL_READ
        | "read_file"
        | tools::TOOL_GREP
        | tools::TOOL_FIND
        | tools::TOOL_LS
        | "list_dir"
        | tools::TOOL_TOOL_SEARCH
        | tools::TOOL_GET_SETTINGS
        | tools::TOOL_AGENTS_LIST
        | tools::TOOL_RECALL_MEMORY
        | tools::TOOL_MEMORY_GET
        | tools::TOOL_JOB_STATUS
        | tools::TOOL_SESSIONS_LIST
        | tools::TOOL_SESSION_STATUS
        | tools::TOOL_SESSIONS_SEARCH
        | tools::TOOL_SESSIONS_HISTORY
        | tools::TOOL_PEEK_SESSIONS => WorkflowEffectClass::Pure,
        _ => WorkflowEffectClass::NonIdempotent,
    }
}

fn task_id_from_args(args: &Value) -> Result<i64> {
    let task = args
        .get("task")
        .ok_or_else(|| anyhow!("workflow.task.update requires task handle from task.create"))?;
    if let Some(id) = task.as_i64() {
        return Ok(id);
    }
    if let Some(id) = task.as_str().and_then(|s| s.parse::<i64>().ok()) {
        return Ok(id);
    }
    task.get("id")
        .and_then(Value::as_i64)
        .ok_or_else(|| anyhow!("workflow.task.update task handle must include id"))
}

fn required_string(args: &Value, key: &str) -> Result<String> {
    optional_string(args, key).ok_or_else(|| anyhow!("missing required string field '{}'", key))
}

fn optional_string(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
}

fn optional_u64_any(args: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter()
        .find_map(|key| args.get(*key).and_then(Value::as_u64))
}

fn compact_input(value: Value) -> Value {
    value
}

#[derive(Debug, Clone, Default)]
struct WorkflowSessionContext {
    working_dir: Option<String>,
    agent_id: Option<String>,
    session_mode: crate::permission::SessionMode,
    project_id: Option<String>,
    incognito: bool,
}

fn workflow_session_context(db: &SessionDB, session_id: &str) -> WorkflowSessionContext {
    let row = {
        let conn = match db.conn.lock() {
            Ok(conn) => conn,
            Err(err) => {
                crate::app_warn!(
                    "workflow",
                    "resolve_root",
                    "session {} lookup lock failed while resolving workflow root: {}",
                    session_id,
                    err
                );
                return WorkflowSessionContext {
                    working_dir: current_dir_string(),
                    ..Default::default()
                };
            }
        };
        conn.query_row(
            "SELECT working_dir, project_id, agent_id, permission_mode, incognito FROM sessions WHERE id = ?1",
            params![session_id],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                ))
            },
        )
        .optional()
    };

    match row {
        Ok(Some((working_dir, project_id, agent_id, permission_mode, incognito))) => {
            let resolved_working_dir = working_dir
                .filter(|s| !s.trim().is_empty())
                .or_else(|| project_id.as_deref().and_then(workflow_root_for_project))
                .or_else(current_dir_string);
            WorkflowSessionContext {
                working_dir: resolved_working_dir,
                agent_id: agent_id.filter(|s| !s.trim().is_empty()),
                session_mode: permission_mode
                    .as_deref()
                    .map(crate::permission::SessionMode::parse_or_default)
                    .unwrap_or_default(),
                project_id,
                incognito: incognito.unwrap_or(0) != 0,
            }
        }
        Ok(None) => WorkflowSessionContext {
            working_dir: current_dir_string(),
            ..Default::default()
        },
        Err(err) => {
            crate::app_warn!(
                "workflow",
                "resolve_root",
                "session {} lookup failed while resolving workflow root: {}",
                session_id,
                err
            );
            WorkflowSessionContext {
                working_dir: current_dir_string(),
                ..Default::default()
            }
        }
    }
}

fn workflow_root_for_project(project_id: &str) -> Option<String> {
    if let Some(db) = crate::get_project_db() {
        match db.get(project_id) {
            Ok(Some(project)) => {
                if let Some(wd) = project.working_dir.filter(|s| !s.trim().is_empty()) {
                    return Some(wd);
                }
            }
            Ok(None) => {}
            Err(err) => {
                crate::app_warn!(
                    "workflow",
                    "resolve_root",
                    "project {} lookup failed while resolving workflow root: {}",
                    project_id,
                    err
                );
            }
        }
    }
    let ws = crate::paths::project_workspace_dir(project_id).ok()?;
    crate::util::ensure_dir_canonical(&ws).ok()
}

fn current_dir_string() -> Option<String> {
    std::env::current_dir()
        .ok()
        .map(|p| p.to_string_lossy().to_string())
}
