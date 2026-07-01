# LSP 与语义代码智能

> 返回 [文档索引](../README.md)
>
> 状态：Phase 3.2 已实现。本文是 `ha-core::lsp`、`lsp` 工具、diagnostics prompt 后缀与 Workspace 诊断面板的单一技术事实源。

## 1. 目标

LSP 子系统让 Hope 不只依赖 `grep/find/read` 的字符匹配，而能通过 Language Server Protocol 获取代码语义：

- definition / references / implementation / hover。
- document symbols / workspace symbols / call hierarchy。
- diagnostics：错误、警告、信息、hint。
- 文件修改后自动同步 `didOpen` / `didChange` / `didSave`，让下一轮对话能看到最新诊断。

设计参考：

- Claude Code 插件文档中的 LSP server 配置模型：`.lsp.json` / `plugin.json`、`command`、`args`、`extensionToLanguage`、diagnostics 默认开启、项目 trust 后启动。
- OpenAI Codex IDE 扩展的上下文方向：打开文件与选区作为 prompt context，语义导航由工具按需拉取。
- LSP 3.17 协议：JSON-RPC、`initialize` capability exchange、`textDocument/*` 与 `workspace/*` 请求。

## 2. 范围

本阶段实现的是产品级第一版 LSP 控制面：

- Core：`crates/ha-core/src/lsp.rs`。
- Agent 工具：`lsp`，注册在 core tools 中。
- Owner API：Tauri `get_lsp_status` / `get_lsp_diagnostics`，HTTP `/api/sessions/{sid}/lsp/status` / `/api/sessions/{sid}/lsp/diagnostics`。
- GUI：Workspace 面板的“语义诊断”区块。
- Prompt：`AssistantAgent::build_merged_system_prompt` 动态追加 `# LSP Diagnostics`。
- File tools：`write` / `edit` / `apply_patch` 成功写入后触发有界同步。

非目标：

- 不内置安装 language server；只检测 PATH 中是否可用。
- 不在无痕会话启动 LSP；language server 可能写本地 index/cache。
- 不把 diagnostics 放进静态 system prompt prefix；必须避免破坏 prompt cache。
- 不把 LSP 当安全边界；读写权限仍由工具权限、工作目录与文件工具守卫负责。

## 3. 默认 Server

`default_configs()` 当前内置以下默认映射：

| Server | Command | Args | 扩展名 |
| --- | --- | --- | --- |
| Rust | `rust-analyzer` | 无 | `.rs` |
| TypeScript | `typescript-language-server` | `--stdio` | `.ts` `.tsx` `.js` `.jsx` `.mjs` `.cjs` |
| Python | `pyright-langserver` | `--stdio` | `.py` `.pyi` |
| Go | `gopls` | 无 | `.go` |
| C/C++ | `clangd` | 无 | `.c` `.h` `.cc` `.cpp` `.cxx` `.hpp` `.hh` |

Server 不可用时：

- `status` 会返回 `available=false`。
- 文件级语义请求返回可读错误，提示安装对应 command。
- 自动同步只记录 `app_warn!("lsp", ...)`，不影响文件写入工具的成功结果。

## 4. Runtime 模型

LSP client 是进程内缓存：

```text
(workspace_root, server_id) -> LspClient
```

`workspace_root` 解析规则：

1. 文件路径先取 parent。
2. 在该目录执行 `git rev-parse --show-toplevel`。
3. 若不是 git 仓库，退回 canonical directory。

每个 `LspClient` 持有：

- child process stdin/stdout。
- JSON-RPC request id。
- pending request map。
- open document version map。
- diagnostics map。

diagnostics 另有同步 cache：

```text
DIAGNOSTIC_CACHE: workspace_root -> uri -> Vec<LspDiagnostic>
```

这样 prompt builder 可以在同步路径读取 compact diagnostics，不需要在 async runtime 里阻塞等待 LSP。

## 5. JSON-RPC 行为

启动流程：

```text
spawn server
-> initialize
-> initialized
-> workspace/didChangeConfiguration
```

请求超时：

- `REQUEST_TIMEOUT_SECS = 8`。
- 超时会移除 pending request 并返回错误。

文件同步：

```text
第一次 sync -> textDocument/didOpen
后续 sync -> textDocument/didChange
did_save=true -> textDocument/didSave
```

写入工具后的自动同步是有界等待：

- 最多等待 3 秒。
- 同步后等待 `SYNC_DIAGNOSTIC_SETTLE_MS = 350ms`，给 server 推送 diagnostics 的窗口。
- 失败只记录 warning。

## 6. Agent 工具

工具名：`lsp`。

动作：

| action | 必要参数 | 返回 |
| --- | --- | --- |
| `status` | 无 | workspace root、server 可用性、active/open docs/diagnostic files |
| `sync_file` | `path` | 同步文件并返回该文件 diagnostics |
| `diagnostics` | 可选 `path` | 当前 workspace 或指定文件 diagnostics |
| `definition` | `path` `line`，可选 `column` | 归一化 location + raw result |
| `references` | `path` `line`，可选 `column` | 归一化 location list + raw result |
| `hover` | `path` `line`，可选 `column` | hover text + raw result |
| `implementation` | `path` `line`，可选 `column` | 归一化 location list + raw result |
| `document_symbols` | `path` | symbol tree/list |
| `workspace_symbols` | 可选 `query` | 每个 active/available server 的 workspace symbols |
| `call_hierarchy` | `path` `line`，可选 `column` / `direction` | incoming/outgoing calls |

`line` / `column` 对模型暴露为 1-based。LSP 内部转换为 0-based position。

## 7. Prompt 注入

`AssistantAgent::build_merged_system_prompt` 在动态后缀区追加：

```text
# LSP Diagnostics
...
```

约束：

- 无痕会话不注入。
- 只有当前 session 有 working dir 且 cache 中有 diagnostics 才注入。
- 最多 `MAX_PROMPT_DIAGNOSTICS = 12`。
- 按 severity 排序，错误优先。
- 文案明确 diagnostics 是代码智能数据，不是用户指令。
- 放在 merged prompt 动态尾部，不进入静态 prefix。

## 8. GUI 交互

Workspace 面板新增“语义诊断”区块，位置在“环境”之后、“进度/Workflow”之前。

显示内容：

- active/available server 数。
- diagnostic files 数。
- error/warning 状态 pill。
- workspace root。
- 最近 6 条 diagnostics：文件、行列、severity、source、message。
- 可手动刷新。

刷新触发：

- 首次打开面板。
- 当前 turn 从 active 变为 idle。
- EventBus 收到 `lsp:diagnostics`。
- EventBus `_lagged` 后兜底刷新。

HTTP 与 Tauri 通过同一 transport command 名称读取，保证 server 模式和桌面模式一致。

## 9. EventBus

LSP server 推送 `textDocument/publishDiagnostics` 时，Core emit：

```text
lsp:diagnostics
```

payload：

```json
{
  "server": "rust-analyzer",
  "workspaceRoot": "/repo",
  "uri": "file:///repo/src/lib.rs",
  "count": 2,
  "diagnostics": []
}
```

该事件只作为 UI 刷新信号和实时状态信号；真实快照仍从 owner API 读取，避免事件丢失导致 UI 状态不完整。

## 10. 安全与隐私

- 无痕会话直接禁用 LSP 工具和自动 sync。
- LSP 子进程继承本地项目上下文，可能写 `.cache` / target index；因此不用于 incognito。
- HTTP owner API 只按 session id 读取该 session working dir 的快照；不会暴露任意 path read endpoint。
- LSP 返回内容视为 untrusted code intelligence；prompt 后缀显式声明不是用户指令。
- 语义工具仍受普通工具可见性、Plan Mode、权限系统与 agent tool filter 控制。

## 11. ACP / IDE 边界

Phase 3.2 的稳定边界是：

- IDE/ACP 传来的 open files / selection 应作为 turn context 或 prompt tail，不进入静态 prefix。
- definition / references / hover / symbols 通过 `lsp` 工具按需读取。
- diagnostics 通过 passive diagnostics cache 注入下一轮，并在 Workspace GUI 可见。

当前 ACP 还没有完整双向 client fs/readTextFile 能力；因此本阶段不在 ACP 内自行读取 IDE open files。后续若 ACP 增加 client context envelope，应复用本架构：

```text
open files / selection -> dynamic turn context
symbols/navigation -> lsp tool
diagnostics -> DIAGNOSTIC_CACHE + prompt suffix + Workspace panel
```

## 12. Roadmap

后续增强不改变本阶段契约：

1. 支持项目级 `.hope/lsp.json` 或插件贡献的 server 配置。
2. LSP client restart/backoff 与 health doctor。
3. diagnostics 进入 Goal evidence / Workflow validation summary 的强类型链路。
4. Review Engine 读取 LSP diagnostics 作为 candidate finding 的证据。
5. ACP IDE context envelope：open files、selection、visible diagnostics、active editor URI。
