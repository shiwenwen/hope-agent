# Review Followups — 审查决定但本期不改的问题

> 本文档登记**已被 code review 识别、但当期 PR 决定不修**的问题。每条记录的目的是：让债务可见、可检索、可调度，避免下一次有人撞上同一个问题再重新发现。
>
> 登记规则见 [AGENTS.md](../../AGENTS.md) "Review Followups 登记"段。

## 文档使用方式

- **新增一条 Follow-up**：在最下方"Open"段追加一个 `### F-XXX` 子节，编号递增（不复用），按下方"条目模板"填写。一次提交一个原子条目；多个 review 想法分开记。
- **关闭一条**：把整段从 "Open" 移到底部 "Closed" 段，附 commit / PR 链接和关闭日期；不要原地删除（保留可检索的历史）。
- **不强制顺序**：可以打散在多个版本里慢慢清。
- **不当作 backlog**：这里只放"review 决定不改"的；功能 backlog 放别处（issue tracker / 其他 plan）。

## 条目模板

每条 Follow-up 至少包含：

```
### F-XXX 简短标题

- **来源**：YYYY-MM-DD `<功能名>` PR / `/simplify` review / 手动审查
- **现象**：一两句描述当前是什么样
- **为什么留**：当期不修的具体理由（范围 / 优先级 / 依赖 / 风险）
- **改的话要做什么**：列出涉及文件、需要的设计决策、可能的迁移路径
- **影响面**：当前是否有用户可见的 bug / 安全 / 性能问题；如果只是"不优雅"就明说
- **触发时机建议**：什么场景下应该顺手收掉（例如 "下一次动这块代码时" / "做某某独立重构 PR 时"）
```

---

## Open

### F-001 Tauri 命令错误类型未统一

- **来源**：2026-04-26 本地小模型助手 `/simplify` review
- **现象**：所有 Tauri 命令返回 `Result<T, String>`，每条命令尾巴都重复一行 `.map_err(|e| e.to_string())` 把 `anyhow::Error` 降成 `String`。`#[tauri::command]` 要求返回值实现 `Serialize`，`anyhow::Error` 不实现，所以不能直接 `?`。
- **为什么留**：ha-server 那边已经有等价的 [`AppError`](../../crates/ha-server/src/error.rs) + `impl<E> From<E> for AppError where E: Into<anyhow::Error>` 让 `?` 直接 work；Tauri 这边没有，统一要动几百条命令的签名 + `invoke_handler!` 注册。属于独立"统一错误类型"重构 PR 的范畴，本期不在 scope。
- **改的话要做什么**：
  1. 在 [`src-tauri/src/commands/mod.rs`](../../src-tauri/src/commands/mod.rs) 引入 `pub struct CmdError(pub String);`
  2. 给 `CmdError` 加 `impl<E> From<E> for CmdError where E: Into<anyhow::Error>`
  3. 给 `CmdError` 加 `impl Serialize`（serialize 成纯字符串，与 `Result<T, String>` 在 IPC wire 上等价，前端零迁移）
  4. 把 [`src-tauri/src/lib.rs`](../../src-tauri/src/lib.rs) 注册的所有命令的返回类型从 `Result<T, String>` 换成 `Result<T, CmdError>`
  5. 删掉所有 `.map_err(|e| e.to_string())`、`.map_err(|e| format!("..."))`，改用 `?`
- **影响面**：纯代码整洁度，无功能 bug、无性能影响。
- **触发时机建议**："Tauri 命令错误类型统一" 独立 PR；或当某条命令需要返回结构化错误（带 code / category）时连带做。

---

### F-002 Provider 写入路径未单一化（add_provider 缺 upsert 语义）

- **来源**：2026-04-26 本地小模型助手 `/simplify` review
- **现象**：仓库里有两条创建 Provider 的路径：
  - [`src-tauri/src/commands/provider/crud.rs::add_provider`](../../src-tauri/src/commands/provider/crud.rs)：GUI"添加 Provider"走这条，**无 upsert**，每次都 `store.providers.push()`
  - [`crates/ha-core/src/local_llm/mod.rs::ensure_ollama_provider_with_model`](../../crates/ha-core/src/local_llm/mod.rs)：本地小模型助手走这条，**有 upsert**（按 base_url 去重）
- **为什么留**：
  1. ha-core 不能反向依赖 src-tauri（AGENTS.md "零 Tauri 依赖" 约束），不能直接调用 `add_provider`
  2. 老的 GUI `add_provider` 不 upsert 不算 bug（用户视觉判重）；只有自动化路径才需要 upsert
  3. 统一要下放 Provider CRUD 到 ha-core，跨多个 crate + 改写入语义，独立工作
- **改的话要做什么**：
  1. 在 [`crates/ha-core/src/provider/`](../../crates/ha-core/src/provider/) 新建 `crud.rs`，把 Provider 写入逻辑（add / update / delete / reorder / upsert_by_base_url）下放，统一走 `mutate_config(("providers.<op>", source), ...)`
  2. [`src-tauri/src/commands/provider/crud.rs`](../../src-tauri/src/commands/provider/crud.rs) 与 [`crates/ha-server/src/routes/providers.rs`](../../crates/ha-server/src/routes/providers.rs) 改成薄壳直接调用 ha-core 函数
  3. `local_llm::ensure_ollama_provider_with_model` 改用统一的 `upsert_by_base_url` helper
- **影响面**：当前无 bug；未来若 `add_provider` 增加副作用（如自动写 SSRF trusted_hosts、emit 特定事件），新代码不会自动享受到，是漂移风险。
- **触发时机建议**：下一次有人需要"再加一条 Provider 创建路径"时（例如 import / batch onboarding），顺势统一；或独立 "Provider CRUD 单一入口" 重构 PR。

---

### F-003 "local Ollama" 判定逻辑分散在 4 处

- **来源**：2026-04-26 本地小模型助手 `/simplify` review
- **现象**：判断"这是不是本地 Ollama"的代码散落在四个文件，正则/字符串/常量各一份：
  - [`crates/ha-core/src/local_llm/mod.rs::is_local_ollama_url`](../../crates/ha-core/src/local_llm/mod.rs) — 后端 upsert 去重
  - [`src/components/settings/ProviderSettings.tsx::LOCAL_OLLAMA_HOST_RE`](../../src/components/settings/ProviderSettings.tsx) — 前端"是否挂载本地小模型卡片"判定
  - [`crates/ha-core/src/openclaw_import/providers.rs`](../../crates/ha-core/src/openclaw_import/providers.rs) — 写死 `http://127.0.0.1:11434`
  - [`src/components/settings/provider-setup/templates/local.ts`](../../src/components/settings/provider-setup/templates/local.ts) — Provider 模板字段
- **为什么留**：跨前后端、跨 crate 的"小常量统一"价值有限；当前四处行为一致没有 bug。强行抽出 shared 常量需要做 wire-format 同步（前端 TS const 怎么从 Rust 同步），引入新约束。
- **改的话要做什么**：候选方案：
  - **A**：在 [`crates/ha-core/src/provider/local.rs`](../../crates/ha-core/src/provider/local.rs)（新建）放 `pub const LOCAL_OLLAMA_BASE_URL` + `pub fn is_local_ollama_url`，后端三处都用；前端继续维护一个独立常量但加注释指向后端同名定义
  - **B**：把"已知本地后端"做成数据驱动表（Ollama / LM Studio / vLLM / SGLang / LiteLLM），后端暴露 `GET /api/local-llm/known-backends`，前端跟着拉
- **影响面**：纯整洁度，没有 bug。但如果有一天 Ollama 默认端口变了或要支持 `http://[::1]:11434` 之类，要改 4 个地方。
- **触发时机建议**：下一次需要在多处加新的 local backend（例如增加对 LM Studio 的一键安装支持）时统一；或者发现端口/host 变化要改时被动收掉。

---

### F-004 NDJSON 流式解析无统一 helper

- **来源**：2026-04-26 本地小模型助手 `/simplify` review
- **现象**：[`crates/ha-core/src/local_llm/mod.rs::pull_model`](../../crates/ha-core/src/local_llm/mod.rs) 用 `bytes_stream` + `buf.iter().position(b'\n')` 手写 NDJSON 切行；同模式在 [`docker/deploy.rs`](../../crates/ha-core/src/docker/deploy.rs)（用 `BufReader::lines()`）、[`mcp/client.rs`](../../crates/ha-core/src/mcp/client.rs)、[`channel/process_manager.rs`](../../crates/ha-core/src/channel/process_manager.rs)、[`agent/providers/anthropic_adapter.rs`](../../crates/ha-core/src/agent/providers/anthropic_adapter.rs)（SSE 流）等处都有 inline 实现，**仓库目前没有统一的"流式 NDJSON helper"约定**。
- **为什么留**：现有几处实现差异较大（reqwest `bytes_stream` vs `tokio::io::BufReader` vs `mpsc::Receiver<String>`），抽通用 helper 需要先统一 input 抽象，跨模块改动大。本期 `pull_model` 的实现已加 1 MiB 单行上限防御 + 单元测试覆盖，质量上可独立。
- **改的话要做什么**：
  1. 在 [`crates/ha-core/src/util.rs`](../../crates/ha-core/src/util.rs)（或新建 `util/ndjson.rs`）加 `pub fn ndjson_stream<S>(stream: S, max_line_bytes: usize) -> impl Stream<Item = Result<Value>>`
  2. 把 reqwest `bytes_stream::Stream<Item = Result<Bytes, _>>` 转 `AsyncBufRead`（用 `tokio_util::io::StreamReader`），再用 `lines()` 拆行 + `serde_json::from_str`
  3. 替换 5 处 inline 实现
- **影响面**：当前无 bug，每处实现都正确但重复。
- **触发时机建议**：下一次新增"还需要解析流式 NDJSON / SSE"的接入点时，顺手抽 helper；不必单独立 PR。

---

### F-005 前端字节/容量格式化在 6+ 处重复

- **来源**：2026-04-26 本地小模型助手 `/simplify` review
- **现象**：前端"把 MB / bytes 格式化成 'X.X GB'"的小函数散落在至少 6 个文件，本期新增 [`LocalLlmAssistantCard.tsx::formatGb` + `formatSize`](../../src/components/settings/local-llm/LocalLlmAssistantCard.tsx) 是第 7 处。其它已知点：
  - [`src/components/settings/dashboard/types.ts`](../../src/components/dashboard/types.ts)
  - [`src/components/settings/BrowserPanel.tsx`](../../src/components/settings/BrowserPanel.tsx)
  - [`src/components/chat/message/FileCard.tsx`](../../src/components/chat/message/FileCard.tsx)
  - [`src/components/log-panel/constants.ts`](../../src/components/log-panel/constants.ts)
  - `SystemMetricsSection.tsx`
- **为什么留**：每处函数都很短（3-5 行），抽到 `src/lib/format.ts` 价值有限；存量债，本期不展开。
- **改的话要做什么**：
  1. 在 [`src/lib/format.ts`](../../src/lib/format.ts)（新建）加 `formatBytes(bytes: number, opts?)` + `formatBytesFromMb(mb: number)`
  2. 全仓 grep `\.toFixed.*GB|toFixed.*MB` 替换调用点
- **影响面**：纯整洁度。语义差异极小，不会引入 bug。
- **触发时机建议**：下一次新加 file-size / capacity 显示组件时顺手抽；或者独立"前端 utility 整理"小 PR 一次清掉。

---

### F-006 Ollama pull 流提前结束时仍会激活模型

- **来源**：2026-04-26 commit `a29a4b27393eb573110e1bafe8f9c0cad11d59c9` review
- **现象**：[`crates/ha-core/src/local_llm/mod.rs::pull_model`](../../crates/ha-core/src/local_llm/mod.rs) 在 `/api/pull` 流结束后，如果最后状态不是 `success`，只打 `app_warn!`，仍返回 `Ok(())`。随后 `pull_and_activate` 会继续注册并激活该模型。
- **为什么留**：本次按 review 结果登记待办，暂不在当前改动里修；修复需要顺手调整 NDJSON 尾部 buffer 处理和错误路径测试，适合作为独立小补丁。
- **改的话要做什么**：
  1. 在 `pull_model` 结束前处理没有换行但仍残留在 buffer 中的最后一行
  2. 将"未收到终态 `success`"从 warn 改成 `Err`
  3. 增加单元测试覆盖 early EOF / truncated frame / final success 三种路径
- **影响面**：用户可见 bug。网络中断、Ollama 进程异常退出或流被截断时，UI 可能提示完成并把一个未完整下载的模型设为 active，下一次聊天才失败。
- **触发时机建议**：下一次动本地小模型 pull 流程时优先修；也可以单独开一个 "Ollama pull completion hardening" PR。

---

### F-007 Ollama 安装成功后进度弹窗不会关闭

- **来源**：2026-04-26 commit `a29a4b27393eb573110e1bafe8f9c0cad11d59c9` review
- **现象**：[`LocalLlmAssistantCard.tsx::installOllama`](../../src/components/settings/local-llm/LocalLlmAssistantCard.tsx) 在 `local_llm_install_ollama` 成功后只 `setDialogDone(true)` + `refresh()`，没有 `setDialogOpen(false)`；而 [`InstallProgressDialog`](../../src/components/settings/local-llm/InstallProgressDialog.tsx) 是受控 `open={dialogOpen}`，也没有 `onOpenChange`。
- **为什么留**：本次按 review 结果登记待办，暂不在当前改动里修；修复很小，但需要决定成功/失败后的弹窗交互（自动关闭、显示完成按钮、允许关闭错误态）。
- **改的话要做什么**：
  1. 安装成功后像模型安装流程一样短暂展示完成态，再关闭弹窗
  2. 给 `InstallProgressDialog` 增加受控 `onOpenChange` 或显式关闭按钮，让 done/error 态可退出
  3. 回归验证安装成功后主卡片能继续展示 "Start Ollama" 操作
- **影响面**：用户可见 bug。安装脚本成功后 modal 仍覆盖设置页，用户会被卡住，无法继续点击启动 Ollama。
- **触发时机建议**：下一次动本地小模型安装向导 UI 时修；也可以和 F-006 一起做成小型 bugfix PR。

---

### F-008 HTTP 模式下手动下载 Ollama 按钮无效

- **来源**：2026-04-26 commit `a29a4b27393eb573110e1bafe8f9c0cad11d59c9` review
- **现象**：[`LocalLlmAssistantCard.tsx::openDownloadPage`](../../src/components/settings/local-llm/LocalLlmAssistantCard.tsx) 先调用 `getTransport().call("open_url")`，失败才 fallback 到 `window.open`。HTTP transport 会把它映射到 [`/api/desktop/open-url`](../../crates/ha-server/src/routes/desktop.rs)，该端点在 server mode 返回 200 + `{ ok: false }`，所以 Promise 正常 resolve，fallback 不会执行。
- **为什么留**：本次按 review 结果登记待办，暂不在当前改动里修；修复需要明确 transport 层 desktop-only API 的统一约定（返回 200 no-op 还是抛错）。
- **改的话要做什么**：
  1. 在前端检查 `open_url` 返回值的 `ok === false` 并主动 `window.open`
  2. 或者让 HTTP transport 对 desktop-only no-op 响应抛错，统一触发现有 fallback
  3. 验证 Windows / HTTP server 模式下 "Download Ollama" 能打开 `https://ollama.com/download`
- **影响面**：用户可见 bug。Windows 用户和远程 HTTP 模式用户点击下载按钮没有任何效果，无法从向导继续安装 Ollama。
- **触发时机建议**：下一次动 HTTP transport desktop-only API 或本地小模型 Windows 分支时修；也可以作为独立前端小修。

---

## Closed

> 已修复条目移到此处，附 commit hash + 关闭日期。保留以便后续 grep。

_(暂无)_
