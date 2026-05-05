# 飞书业务 API 工具化 — docx / bitable / drive / wiki

> 目标：把飞书除 IM 之外的核心业务 API（云文档 docx、多维表格 bitable、云盘 drive、知识库 wiki）做成 hope-agent 的 internal tools，让 agent 能"读飞书表"、"改飞书文档"、"上传到飞书云盘"。
>
> 这是飞书全面对齐的最大块工作量，相对独立于 [Phase A 协议层补完](../../crates/ha-core/src/channel/feishu/)（已合并）和 [Phase B 入站事件扩展](channel-inbound-events.md)（待启动）。

## 0. Context

### 0.1 现状

[`channel/feishu/api.rs`](../../crates/ha-core/src/channel/feishu/api.rs) 当前只覆盖 IM 收发（send_message / send_interactive_card / send_image / upload_image / upload_file / update_message / delete_message / get_bot_info / get_ws_endpoint）—— bot 收发消息够用，但 agent 想做"读用户的多维表格"、"改文档"完全做不到。

openclaw 的飞书插件 [`extensions/feishu/src/`](../../../openclaw/extensions/feishu/src/) 通过 SDK 接通了 docx / bitable / drive / wiki / approval 等业务面，作为独立 plugin 暴露。hope-agent 没有 plugin 加载机制（[`tools/definitions/registry.rs`](../../crates/ha-core/src/tools/definitions/registry.rs)），但内置 tools 就能覆盖这些场景——agent 不需要"飞书是个 plugin"，它需要"我能调 feishu_docx_get_blocks 这种 tool"。

### 0.2 目标

| # | 目标 | 验收 |
|---|------|------|
| G1 | docx / bitable / drive / wiki 的核心 read/write API 可被 agent 调用 | MVP 范围内 12 个 tool 全部能在 hope-agent 桌面 / server 模式 invoke |
| G2 | 凭据零额外配置 | 用户已配 IM channel 即可用，不需要重复填 app_id/app_secret |
| G3 | 同 SSRF / 限流 / 错误处理路径 | 全部走现有 `FeishuApi` 客户端 + 现有 [`security::ssrf::check_url`](../../crates/ha-core/src/security/) 拦截 |
| G4 | 单元测试覆盖每个 endpoint 的 happy/error path | mock reqwest server 验证请求拼装 + 响应解析 |
| G5 | settings 风险等级登记到 [`ha-settings` 技能文档](../../skills/ha-settings/SKILL.md) | 每 tool 一行，与 [tools/settings.rs](../../crates/ha-core/src/tools/settings.rs) 同步 |

### 0.3 非目标

| # | 不做 | 原因 |
|---|---|---|
| N1 | 全量对齐飞书 SDK（数百 endpoint） | 工作量爆炸；MVP 12 个覆盖 70% 用例足够，后续按 issue 增量补 |
| N2 | approval / hire / contact / calendar / task / email 等子系统 | 飞书"业务 API"语义太宽，本计划只锁定 docx/bitable/drive/wiki 四块——与"用户日常用 hope-agent 的 agent 处理飞书内容"高频场景对齐 |
| N3 | 把业务 API 独立成新 channel / 新 crate | 复用现有 `FeishuApi` 客户端 + tools 静态注册，避免架构膨胀 |
| N4 | 给业务 API tool 接 deferred 加载池 | 首版直接 always_load；如果未来这 12 个 tool 占 prompt 太多，再升级到 deferred |
| N5 | 飞书国际版（Lark）特殊字段补全 | 沿用现有 `FeishuAuth::resolve_base_url` 已支持的 feishu/lark/自定义 domain；endpoint URL 路径在两域名下相同 |

## 1. 架构决策

### 1.1 凭据来源（必须先定）

[Explore agent 摸到的现状](#1.4)：[`tools/definitions/registry.rs`](../../crates/ha-core/src/tools/definitions/registry.rs) tools 静态注册，无 plugin 加载口子；`FeishuAuth` 在 [`channel/feishu/auth.rs`](../../crates/ha-core/src/channel/feishu/auth.rs) 是 channel 私产。

**三种方案对比**：

| 方案 | 优点 | 缺点 | 评估 |
|------|------|------|------|
| **A. 复用 channel registry 拿 auth**（推荐） | 凭据零额外配置；用户配过 IM channel 就能用业务 tool | 业务 tool 必须先开 IM channel（即使用户只想用 docx 不接 bot） | ✅ 选这个 |
| B. AppConfig 单开 `feishu_credentials` | 与 channel 解耦，纯文档场景免开 channel | 用户得配两次同样的 app_id/secret；凭据散落 | ❌ |
| C. 同时支持，按 priority | 灵活 | 复杂；两套配置 UI；同步更新心智负担高 | ❌（首版不要） |

**选 A**。代价：用户即使只用 docx tool 也要先配 IM channel（不一定 start，配好凭据通过 `validate_credentials` 即可——或者一个 placeholder bot）。可以接受——多数飞书业务用户本来就有 bot。

### 1.2 多账号路由

用户可能配多个飞书 IM channel 账号（不同公司 / 个人 vs 工作）。tool 调用时要让 agent 选择走哪个账号：

```rust
// tool 参数加一个可选 `account` 字段
{
    "type": "object",
    "properties": {
        "account": {
            "type": "string",
            "description": "Feishu channel account ID. Defaults to the only configured account if exactly one exists."
        },
        "document_id": { "type": "string" },
        ...
    }
}
```

实现：tools/feishu/mod.rs 的 helper `resolve_feishu_api(account: Option<&str>) -> Result<Arc<FeishuApi>>`：

- `Some(id)` → `FeishuPlugin::get_account(id)`
- `None` → 枚举所有 feishu 账号，恰好 1 个则用之；多个则错误"必须指定 account"
- 0 个则错误"飞书 channel 未配置"

### 1.3 工具分级（Tier 选择）

参考 [`tools/definitions/types.rs::ToolTier`](../../crates/ha-core/src/tools/definitions/types.rs#L16):

- ❌ Tier 1 Core：强制注入；不合适——飞书 tool 不是 hope-agent 必备
- ✅ **Tier 3 Configured**：需要全局 provider/capability 配置；agent 默认开但没配凭据时进 system prompt `# Unconfigured Capabilities` 段。匹配"配过 IM channel 才生效"的语义
- 默认 `default_for_main = false`、`default_for_others = false` —— 用户主动开（避免污染所有 agent 的 prompt）

`config_hint`：`"Configure a Feishu IM channel account to enable docx / bitable / drive / wiki tools"`。

### 1.4 凭据共享的实现路径

[`channel/feishu/mod.rs:70`](../../crates/ha-core/src/channel/feishu/mod.rs#L70) 现在的 `FeishuPlugin::get_account` 是 `async fn` 私有方法。本计划改为 `pub(crate)` 或加一个 `pub(crate) fn api_for_account(&self, account_id: &str) -> Option<Arc<FeishuApi>>`（同步版 — 内部 lock 时间极短）。

tools 通过 `crate::channel::registry::get_feishu_plugin()` 拿到 plugin 引用，再调上面方法。这要求 [`channel/registry.rs`](../../crates/ha-core/src/channel/registry.rs) 暴露按 ChannelId 查 plugin 的方法（如果还没有的话）。

## 2. MVP 范围（首版 12 个 tool）

按使用频率挑（参考 openclaw 的统计 + 飞书生态调研）：

### 2.1 docx — 4 个

| tool name | endpoint | 用途 |
|---|---|---|
| `feishu_docx_create` | `POST /open-apis/docx/v1/documents` | 新建空文档（返回 document_id） |
| `feishu_docx_get_blocks` | `GET /open-apis/docx/v1/documents/{id}/blocks` | 列出全部 block（支持分页） |
| `feishu_docx_append_block` | `POST /open-apis/docx/v1/documents/{id}/blocks/{block_id}/children` | 在指定 block 末尾追加新 block |
| `feishu_docx_update_block_text` | `PATCH /open-apis/docx/v1/documents/{id}/blocks/{block_id}` | 改 block 内文本（覆盖式） |

### 2.2 bitable — 4 个

| tool name | endpoint | 用途 |
|---|---|---|
| `feishu_bitable_list_records` | `GET /open-apis/bitable/v1/apps/{app_token}/tables/{table_id}/records` | 拉记录（分页+filter） |
| `feishu_bitable_search_records` | `POST .../records/search` | 复杂查询（field_names + filter + sort） |
| `feishu_bitable_create_record` | `POST .../records` | 单条新增 |
| `feishu_bitable_batch_update_records` | `POST .../records/batch_update` | 批量改（最多 1000 条/请求） |

### 2.3 drive — 3 个

| tool name | endpoint | 用途 |
|---|---|---|
| `feishu_drive_list_files` | `GET /open-apis/drive/v1/files` | 列文件夹内容 |
| `feishu_drive_upload_media` | `POST /open-apis/drive/v1/medias/upload_all` | 上传二进制文件（≤20MB；> 20MB 用 v2 分片，下版补） |
| `feishu_drive_download_media` | `GET /open-apis/drive/v1/medias/{file_token}/download` | 下载文件到本地 |

### 2.4 wiki — 1 个

| tool name | endpoint | 用途 |
|---|---|---|
| `feishu_wiki_get_node` | `GET /open-apis/wiki/v2/spaces/get_node` | 用 token 反查文档元信息（找到文档所在 space / parent） |

**为啥 wiki 只一个**：写 wiki 要先知道 space_id；但典型 agent 用例是"用户给我一个 wiki 链接，帮我读内容"——读用 docx_get_blocks（wiki 文档背后是 docx），wiki API 主要是元信息查询。后续按需加 `feishu_wiki_create_node` / `feishu_wiki_list_children`。

## 3. 实现细节

### 3.1 文件结构

```
crates/ha-core/src/tools/feishu/
  mod.rs              # tool 集合入口 + resolve_feishu_api helper + 公共错误处理
  docx.rs             # 4 个 docx tool 的 ToolDefinition + execute fn
  bitable.rs          # 4 个 bitable tool
  drive.rs            # 3 个 drive tool
  wiki.rs             # 1 个 wiki tool
```

每个 file 暴露 `pub fn get_xxx_tool() -> ToolDefinition`，在 `mod.rs` 汇总：

```rust
pub fn get_feishu_tools() -> Vec<ToolDefinition> {
    vec![
        docx::create_tool(),
        docx::get_blocks_tool(),
        docx::append_block_tool(),
        docx::update_block_text_tool(),
        bitable::list_records_tool(),
        bitable::search_records_tool(),
        bitable::create_record_tool(),
        bitable::batch_update_records_tool(),
        drive::list_files_tool(),
        drive::upload_media_tool(),
        drive::download_media_tool(),
        wiki::get_node_tool(),
    ]
}
```

[`tools/definitions/extra_tools.rs`](../../crates/ha-core/src/tools/definitions/extra_tools.rs) 末尾加 `tools.extend(super::super::feishu::get_feishu_tools())`。

### 3.2 API 客户端扩展

[`channel/feishu/api.rs`](../../crates/ha-core/src/channel/feishu/api.rs) 现在只有 IM 方法，太长不再合适塞所有业务 API。拆成：

```
crates/ha-core/src/channel/feishu/
  api.rs              # 现有 IM API（保持）
  api_docx.rs         # docx REST 客户端方法
  api_bitable.rs      # bitable
  api_drive.rs        # drive
  api_wiki.rs         # wiki
```

每个 file 给 `impl FeishuApi` 加一组方法：

```rust
// api_docx.rs
impl FeishuApi {
    pub async fn docx_create(&self, title: Option<&str>) -> Result<String> { ... }
    pub async fn docx_get_blocks(&self, document_id: &str, page_token: Option<&str>) -> Result<DocxBlocksPage> { ... }
    pub async fn docx_append_block(&self, document_id: &str, parent_block_id: &str, block: serde_json::Value) -> Result<()> { ... }
    pub async fn docx_update_block_text(&self, document_id: &str, block_id: &str, text: &str) -> Result<()> { ... }
}
```

所有方法都走现有 `authorized_request` helper（已带 tenant_access_token + 错误响应解析）。

### 3.3 错误处理

每个 tool execute fn 走统一模板：

```rust
pub async fn execute_docx_get_blocks(args: serde_json::Value) -> Result<serde_json::Value> {
    let document_id = args.get("document_id").and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("document_id is required"))?;
    let account = args.get("account").and_then(|v| v.as_str());
    let page_token = args.get("page_token").and_then(|v| v.as_str());

    let api = resolve_feishu_api(account).await?;

    let result = api.docx_get_blocks(document_id, page_token).await?;
    Ok(serde_json::to_value(result)?)
}
```

LLM 看到的错误信息通过 anyhow context 链 + `truncate_utf8` 限长保护（避免飞书 API 错误响应几 KB 撑爆 prompt）。

### 3.4 SSRF / 出站策略

按 [AGENTS.md "SSRF 统一策略"](../../AGENTS.md) 红线，新出站入口必须走 `security::ssrf::check_url`。但 [`channel/feishu/api.rs::authorized_request`](../../crates/ha-core/src/channel/feishu/api.rs) 当前**没**走 ssrf — 因为飞书域名是受信白名单。需在 Phase C 启动时确认这个豁免是否要保留：

- 如果保留：在每个新 api_xxx.rs 顶部 doc 注明 "domain is feishu/lark, exempted from SSRF"
- 如果改：authorized_request 加 `security::ssrf::check_url(&url)?;`，影响所有现有 IM API（要回归测一遍）

**推荐保留豁免**——飞书域名稳定，SSRF 主要防内网/127.0.0.1，这里不适用。但开 PR 时单独发一个 issue/decision doc 备案。

### 3.5 settings 集成

按 AGENTS.md「设置约定」，每个新 tool 要在三处登记：

1. [`src/components/settings/`](../../src/components/settings/) 加 GUI 开关（每 agent 的 capabilities.tools 里勾选）—— 现有 capabilities 配置面板已经支持任意 tool name，无需新组件
2. [`tools/settings.rs`](../../crates/ha-core/src/tools/settings.rs) 不需要加分支（这是给 update_settings 写 settings 的，新 tool 不引入新 settings field）
3. [`skills/ha-settings/SKILL.md`](../../skills/ha-settings/SKILL.md) **风险等级表**加 12 行：所有飞书业务 tool 标 **MEDIUM**（可以读/改用户的飞书云内容，影响范围超出本机但仅限飞书租户内）

### 3.6 limit & 限流

飞书 API 有租户级 QPS 限制（typically 50/s tenant-wide for docx, 20/s for bitable）。本计划**不做**应用层限流——超限时飞书返 99991400 错误码，由 LLM 自行重试或 backoff。如果实战发现限流频繁，下版加：

```rust
// channel/feishu/api.rs 加每账号 RateLimiter
struct RateLimited { permits: Semaphore, last_reset: Instant }
```

但 MVP 不做，避免过度设计。

## 4. PR 切分

每个子模块独立 PR，分批合并降低 review 难度：

### Phase C.0 — 基础设施 PR

**包含**：

1. `tools/feishu/mod.rs` 骨架（`resolve_feishu_api` helper + `get_feishu_tools()` 空 vec 占位）
2. `channel/registry.rs` 暴露 `pub fn get_feishu_plugin()`（如果还没有）
3. `channel/feishu/mod.rs` 把 `get_account` 改为 `pub(crate)` 或加 `api_for_account` 方法
4. `tools/definitions/extra_tools.rs` 末尾接入 `tools.extend(get_feishu_tools())`（接空 vec，不破坏现有行为）
5. 单测：multi-account / no-account / single-account 三种场景的 `resolve_feishu_api`

**不包含**：任何具体业务 tool。

**预估**：0.5 天。

### Phase C.1 — docx 4 个 tool

**包含**：

1. `channel/feishu/api_docx.rs`（4 个 API 方法 + 单测 mock reqwest server）
2. `tools/feishu/docx.rs`（4 个 ToolDefinition + execute fn + 单测）
3. `tools/feishu/mod.rs::get_feishu_tools()` 加 4 个 docx
4. [`skills/ha-settings/SKILL.md`](../../skills/ha-settings/SKILL.md) 风险表加 4 行
5. e2e 实地：在 ShadowAI-Feishu 账号下用 agent 调用 `feishu_docx_create` 看是否真建了文档

**预估**：1 天。

### Phase C.2 — bitable 4 个 tool

同 C.1 模板，针对 bitable。**注意**：bitable 的 record `fields` 是动态结构（用户自定义列），schema 用 `Object` 而非具体字段。

**预估**：1 天。

### Phase C.3 — drive 3 个 tool

**特殊点**：drive 的 upload/download 涉及二进制流。
- upload：tool 参数加 `file_path: string`（本地文件路径），内部 read + multipart upload
- download：tool 参数加 `save_to: string`（本地保存路径），写盘后返回路径而不是把 binary 塞 prompt
- 关注路径校验：本地路径走现有 `permission` engine 走 `Protected/Dangerous` 检查（参考 `tools/exec.rs` 模式）

**预估**：1 天（含路径权限测试）。

### Phase C.4 — wiki 1 个 tool

**包含**：1 个 wiki_get_node + ha-settings 1 行。

**预估**：0.5 天。

### 总工作量

C.0-C.4 串行做 ≈ 4 天。如果有人手并行 C.1/C.2/C.3，2 天可完成。

## 5. 测试策略

### 5.1 单元测试

每个 api_xxx.rs 用 [`mockito`](https://crates.io/crates/mockito) 或 [`wiremock`](https://crates.io/crates/wiremock) 启动 mock HTTP server（hope-agent workspace 当前**未引入**这两个 crate，需要决定是否引），覆盖：

- happy path：构造预期请求 + 返回 mock 响应 + 验证解析
- error path：飞书 code != 0 → 错误 propagate
- 网络错误：reqwest timeout → 错误 propagate

或者简单点：用 `httpmock` / 手写 `axum::Router` 在测试里 spawn 一个 mock server，绑 `127.0.0.1:0` 拿端口然后让 `FeishuApi` 指向它。**推荐 wiremock**——API 简洁，社区主流，不污染依赖（dev-only）。

### 5.2 tool 层测试

每个 `tools/feishu/xxx.rs` 单测：

- args 缺关键字段 → 错误
- args 含 `account: "non-existent"` → resolve 失败
- args 含合法字段 + mock api → execute 返回符合 schema 的 JSON

### 5.3 e2e 实地

每子 PR 合并前在 ShadowAI-Feishu 账号下：

1. C.1：建文档 → 列 block → 追加 block → 改文本 → 通过 GUI 看到改动
2. C.2：建多维表 → list/create/update record → 通过 GUI 看到行
3. C.3：上传一张图 → list 看到 → 下载到本地 → diff 文件大小
4. C.4：用 wiki link 提取 node info → 验证返回结构

实地需要测试账号的 docx / bitable / drive 三个权限——FeishuPlugin 的 `validate_credentials` 现在只查 bot info，不查这些权限。**预先确认**：开发期手动在飞书后台给 ShadowAI-Feishu 的 app 加 `docx:document` / `bitable:app` / `drive:drive` / `wiki:wiki:readonly` 权限范围。

## 6. 风险与边角

### 6.1 多账号枚举

[`channel/registry.rs`](../../crates/ha-core/src/channel/registry.rs) 是否暴露"枚举某 channel 下所有 account_id"的 API？需要 explore agent 二次确认 — 如果没有要新增。

### 6.2 tenant_access_token 并发刷新

[`channel/feishu/auth.rs:60-82`](../../crates/ha-core/src/channel/feishu/auth.rs#L60) 的 `Mutex<Option<CachedToken>>` 已自动刷新；但**多个 tool 并发调用同账号**时，第一个发现过期触发刷新，其它都 await 同一锁——会串行化。生产场景 IM bot 调用频率低，不构成问题；如果飞书 tool 突发 50+ 并发，需要换成 `RwLock` + double-check。MVP 不优化，留为后续 issue。

### 6.3 大响应

`docx_get_blocks` / `bitable_list_records` 单页可能上百条，序列化后几十 KB。结合 [AGENTS.md「工具结果磁盘持久化」](../../AGENTS.md) — > 50KB 自动落盘 + head/tail preview，已经覆盖。**确认**：每个新 tool 的 execute fn return 走的是统一 dispatch 路径，自动享有这个能力——是的，dispatch 在 [`tools/dispatch.rs`](../../crates/ha-core/src/tools/dispatch.rs) 是统一封装的。

### 6.4 失败模式 vs LLM 心智

飞书 API 报错信息一般是中文 + 错误码。LLM 看到 `code=99991400 msg=请求过快` 会自己 retry——不需要我们包装。但 `code=403 msg=权限不足` 这类信息要让 LLM 知道是**租户配置问题**而非"我代码错了"，error message 里加提示：

```rust
return Err(anyhow!(
    "Feishu API permission denied (code={}). Please ensure the bot app has '{}' scope granted in Feishu admin panel.",
    code, scope_hint
));
```

每个 tool 配 scope_hint 字符串。

### 6.5 凭据复用 vs IM 状态

如果用户**只**想用业务 tool 不开 IM bot：

- 配 channel + `validate_credentials` 通过即生成 FeishuApi（凭据存了）
- **不**调 `start_account`（不开 ws 长连接）
- tool 调用时 `resolve_feishu_api` 仍能拿到 `Arc<FeishuApi>`

需要确认 [`channel/feishu/mod.rs::start_account`](../../crates/ha-core/src/channel/feishu/mod.rs#L110) 之前的 `accounts` HashMap 是否仅 `start_account` 时填充——如果是，"不 start"的账号 tool 拿不到 api。

**对策**：把 `accounts` insert 提到 `validate_credentials` 之后立即做（即使不 start）。或者 tool 层走另一个 fallback：直接从 `ChannelAccountConfig` 重新构造 `FeishuApi`。

按当前代码 [`channel/feishu/mod.rs:134-145`](../../crates/ha-core/src/channel/feishu/mod.rs#L134) 的 insert 在 `start_account` 末尾。**Phase C.0 PR 必须改**：抽出 `register_account` 单独函数，`validate_credentials` + `start_account` 都调用它，确保只配凭据不 start 也能让 tool 拿到 api。

### 6.6 Lark 国际版

[`channel/feishu/auth.rs:142-200`](../../crates/ha-core/src/channel/feishu/auth.rs#L142) 已支持 `feishu` / `lark` / 自定义 domain。所有业务 API 路径在两域名下相同（验证：openclaw 用同一份 SDK 跑 lark.com 没问题），无需二次适配。

## 7. 文档维护

按 AGENTS.md 文档维护表，本计划每个子 PR 触发：

| 文件 | 改动 |
|---|---|
| [`CHANGELOG.md`](../../CHANGELOG.md) | 每 PR 加一条"feat(tools): feishu_xxx 等 N 个 tool" |
| [`AGENTS.md`](../../AGENTS.md) | 不需要改契约面，飞书 toolset 不引入新红线 |
| [`docs/architecture/tool-system.md`](../architecture/tool-system.md) | 末尾加"飞书业务 toolset"一节，列 12 个 tool + 凭据模型 |
| [`docs/architecture/api-reference.md`](../architecture/api-reference.md) | 不变（tools 不暴露到 Tauri/HTTP 层，走的是 LLM tool calling） |
| [`README.md`](../../README.md) + [`README.en.md`](../../README.en.md) | 在"Capabilities"部分加一句"Feishu workspace integration (docx/bitable/drive/wiki)" |

## 8. 验收清单

每子 PR 合并前：

- [ ] 该子模块下全部 tool 的 happy/error path 单测过
- [ ] mockito/wiremock e2e 测试过（不依赖真实飞书 API）
- [ ] ha-settings SKILL.md 风险表行已加
- [ ] tool_system.md 飞书一节本子模块对应行已加
- [ ] 实地一次：用真实 ShadowAI-Feishu 账号在桌面 dev 调一次 → 看真实飞书后台改动生效
- [ ] CHANGELOG.md 已加条目
- [ ] grep 验证：tool name 命名一致 `feishu_<module>_<verb>`，schema 字段都用 snake_case

C.0-C.4 全合并后做一次 final 实地：让 agent 跑一个真实工作流（"读多维表→生成 docx 周报→上传到云盘"）确认端到端可用。

## 9. 后续路线

MVP 之后按 issue 增量补：

| 优先级 | 项目 | 备注 |
|---|---|---|
| P1 | docx 块类型扩展 | 当前只 update_block_text；后续接 image/table/code block |
| P1 | drive 大文件分片上传（>20MB） | 用 v2 upload_prepare/upload_part/upload_finish |
| P2 | bitable view / dashboard | 改/查视图、看板 |
| P2 | approval（审批） | 创建审批实例、查询、撤销 |
| P3 | calendar / contact / hire | 各自体系，按真实需求驱动 |
| P3 | 限流 RateLimiter | 实战出现 99991400 频繁时再做 |
