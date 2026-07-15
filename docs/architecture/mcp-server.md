# 平台级 MCP Server（`hope-agent mcp`）

> 返回 [文档索引](../README.md)
>
> Hope Agent **当 MCP server**：把子系统能力经标准 MCP 协议暴露给本机外部 agent（Claude Code /
> Cursor 等）。这是「Hope Agent as MCP server」平台议题的第一块砖——共享 stdio host +
> `ToolProvider` 注册表，**设计空间（design）是首个 provider**。
>
> 与 [`mcp.md`](mcp.md) 区分：那是 MCP **客户端**（我们连别人的 server）；本文是我们**当 server**。
> MCP 规范里 "host" 指客户端宿主应用，故 server 侧模块命名为 `mcp_server`（不叫 `mcp_host`）。

## 1. 形态与红线

- **入口**：`hope-agent mcp`（双 bin 都接线：`src-tauri/src/main.rs` + `crates/ha-server/src/bin/hope-agent.rs`）。stdio、本机信任、无 token（进程由本机 owner 直接 spawn，等同 owner 平面）。
- **不做子系统专属 server（红线）**：把 Hope Agent 暴露成 MCP server 是**平台议题**；design 只是 host 上的一个 provider，**不自起 server**。`knowledge-mcp` 是更早的独立子命令，保持原样不动（内部归并进共享循环记 P+1，届时 serverInfo/CLI 不变）。
- **写门双保险**：默认只读；`--allow-writes` 才暴露写工具。host 在 `tools/call` 层再拦一次——即使 provider 忘了在 `tools()` 里裁剪写工具，只读模式调用写工具一律拒。
- **runtime 红线**：`run_stdio` 建 **multi_thread**（2 worker）runtime。provider 写工具（如 design 生成）内部 `tokio::spawn` 后台任务，current_thread runtime 在 `block_on` 返回后不再驱动 spawned task 会让后台生成僵死。**切勿「优化」回 current_thread**（模块文档 + 单测钉死）。
- **无会话轴**：MCP 面直调 service、无 `session_id`，故**无 incognito 语义**；写门只有 `--allow-writes`。

## 2. 协议（`crates/ha-core/src/mcp_server/mod.rs`）

- newline-delimited JSON-RPC 2.0 over stdio，`PROTOCOL_VERSION = "2025-03-26"`（对齐 `knowledge::agent_mcp`）。
- 方法：`initialize`（`serverInfo.name="hope-agent"` + 各 enabled provider 的 `instructions` 拼接）/ `ping` / `notifications/initialized`（无响应）/ `tools/list`（enabled provider ∧ 写门裁剪）/ `tools/call`（名字精确分发 + host 写门 + `isError` 封装）/ `resources|prompts/list`（空）。错误码：`-32700` parse / `-32600` 缺 method / `-32601` 未知 method；工具错误走 `isError:true` 文本。

## 3. `ToolProvider` 契约

```rust
pub struct McpCtx<'rt> { pub allow_writes: bool, pub runtime: &'rt tokio::runtime::Runtime }
pub struct ToolDef { pub name: &'static str, pub description: String, pub input_schema: Value, pub read_only: bool }
pub trait ToolProvider: Send + Sync {
    fn name(&self) -> &'static str;
    fn enabled(&self) -> bool { true }               // 配置门；false → list 不含 + call 拒
    fn instructions(&self) -> Option<&'static str> { None }
    fn tools(&self, ctx: &McpCtx) -> Vec<ToolDef>;   // provider 自行按 ctx.allow_writes 裁剪写工具
    fn call(&self, name: &str, args: Value, ctx: &McpCtx) -> Result<Value>;  // 同步 + block_on，不引 async-trait
}
```

工具名前缀约定 `<provider>_`（如 `design_list_projects`）；`enabled()` 双面 fail-closed（disabled 时 tools/list 无 + tools/call 拒）。

## 4. design provider（`crates/ha-core/src/design/mcp_provider.rs`）

全部薄包 `crate::design::service`（owner 平面），与 HTTP / Tauri 平级复用、零新逻辑。`enabled()` 读 `cached_config().design.enabled`。

**读集（恒可见，8）**：`design_list_projects` / `design_list_artifacts`(projectId required——无会话故不走 `get_or_create` 防新建草稿项目) / `design_get_artifact`（`get_artifact_view` + `get_artifact_source_for_agent`(oid 源码) + `list_comments` 组合；`status=="generating"` 且 `updated_at` 落后 >600s 附 `maybeOrphaned`——孤儿对账只在桌面产物墙跑）/ `design_get_active_context` / `design_list_systems` / `design_get_system`（DESIGN.md + 可选 `tokenFormat` 过滤）/ `design_list_comments` / `design_list_versions`。

**写集（`--allow-writes` 才注册，7）**：`design_generate_artifact`（包 `generate_design_artifact`——HTML 形态立即返 generating 壳、轮询 `design_get_artifact` 至 `status!="generating"`；image/audio/component 同步阻塞）/ `design_update_artifact`（`origin:"ai"`，可选 `expectedBodyHash`）/ `design_edit_element`（`patch_element`；**`expectedBodyHash` schema 层 required**——跨进程无共享 `artifact_lock` 的主动收紧，patch 层锁内重校兜底；`text_node` 不暴露）/ `design_restyle` / `design_restore_version` / `design_add_comment` / `design_resolve_comment`。

**恒不暴露（红线，provider 不定义即不可达）**：`implement_to_code`、代码绑定写、`deploy*`、`share`、`delete_project`、`delete_artifact`、`save_to_knowledge`、`extract_system`（`scoped_local_path` 以会话为根，MCP 无会话无法安全界定读根）、`export_*`（写 Downloads）——外部 agent 不得经 MCP 写用户代码仓库、对外发布或删除容器。

## 5. active-context 事实源

MCP 是无状态新进程、无 GUI 会话，`design_get_active_context` 需要「用户此刻在看什么」的服务端事实源：

- `design_projects` 加 `last_opened_artifact_id` + `last_opened_at` 两列（幂等 ALTER；**不进 `PROJECT_COLUMNS`/DTO/mapper**，专用方法 `set_last_opened`/`last_opened`；design.db 可重建，丢了走 fallback）。
- 前端 `openArtifact` 后台 fire-and-forget `mark_design_artifact_opened_cmd`（失败静默）→ `service::mark_artifact_opened`（**不调 `touch_project`**——浏览≠编辑，不抬 `updated_at` 扰动最近排序）。
- `service::get_active_context()`：`last_opened` 记录（TTL 30min 内新鲜 / 超 TTL 标 `stale`）→ 产物/项目已删则回退最近更新项目 + 其最新产物（`source="recent"`）→ 无项目 `source="none"`。载荷 = project + artifact 摘要（**不内联源码**，另调 `design_get_artifact`）+ open comments 正文 + `CodeBindingInfo` + 最近设计对话 session id。

## 6. 已知风险与限制

- **跨进程写并发**（桌面 + MCP 同开 design.db，`artifact_lock` 进程内）：`design_edit_element` schema 强制 `expectedBodyHash` + patch 层锁内重校兜底缓解；暴露面与 knowledge-mcp 同级、非本轴新增；P+1 候补目录级 advisory file lock。
- **generating 孤儿**：MCP 进程被杀留 generating 壳，仅桌面产物墙 `list_all_artifacts` 对账自愈；`design_get_artifact` 附 `maybeOrphaned` 提示。
- **GUI 无实时刷新**：MCP 进程 emit 的 `design:reload` / `design:code_drift` 不跨进程；用户重开产物即见新内容。
