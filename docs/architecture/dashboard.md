# Dashboard 数据大盘架构
> 返回 [文档索引](../README.md) | 更新时间：2026-04-05

## 概述

Dashboard 模块提供跨三个 SQLite 数据库（SessionDB、LogDB、CronDB）的聚合分析查询，为前端 recharts 图表提供标准化 JSON 数据。模块拆分为 6 个文件，采用「筛选器 + 查询函数」的管道式架构。

核心设计原则：
- **自动排除非用户数据**：所有 session 级查询自动注入 `is_cron = 0 AND parent_session_id IS NULL`，排除定时任务会话和子 Agent 会话
- **统一筛选**：所有查询接受同一个 `DashboardFilter` 结构体，支持时间范围 + Agent/Provider/Model 维度筛选
- **成本估算内联**：Token 统计查询自动附带基于硬编码定价表的 USD 成本估算
- **进程级系统指标**：通过 sysinfo crate 采集当前进程的 CPU/内存/磁盘 IO 实时快照

## 模块结构

| 文件 | 职责 |
|------|------|
| `mod.rs` | 模块入口，re-export 公开 API |
| `types.rs` | 全部数据结构定义（20 个 struct） |
| `queries.rs` | 7 个聚合查询函数 |
| `detail_queries.rs` | 5 个详情列表查询函数 |
| `filters.rs` | 筛选器构建（session / log 两套） |
| `cost.rs` | 模型定价表与成本计算引擎 |

## 数据源架构

```mermaid
graph TB
    subgraph 前端
        RC[recharts 图表组件<br/>src/components/dashboard/]
    end

    subgraph 命令层
        CMD[dashboard_* 命令<br/>src-tauri/src/commands/dashboard.rs<br/>桌面模式 Tauri 命令入口]
    end

    subgraph Dashboard模块
        F[filters.rs<br/>DashboardFilter]
        Q[queries.rs<br/>7 个聚合查询]
        DQ[detail_queries.rs<br/>5 个详情查询]
        C[cost.rs<br/>成本估算引擎]
        T[types.rs<br/>20 个数据结构]
    end

    subgraph 数据源
        SDB[(SessionDB<br/>sessions + messages<br/>+ subagent_runs)]
        LDB[(LogDB<br/>logs)]
        CDB[(CronDB<br/>cron_jobs<br/>+ cron_run_logs)]
        SYS[sysinfo<br/>进程级指标]
    end

    RC -->|invoke| CMD
    CMD --> Q
    CMD --> DQ
    Q --> F
    DQ --> F
    Q --> C
    Q --> SDB
    Q --> LDB
    Q --> CDB
    Q --> SYS
    DQ --> SDB
    DQ --> LDB
    F --> T
```

## 筛选器系统

### DashboardFilter

所有查询的统一入参，5 个可选维度：

| 字段 | 类型 | 说明 |
|------|------|------|
| `start_date` | `Option<String>` | 起始时间（ISO 8601 格式） |
| `end_date` | `Option<String>` | 结束时间（ISO 8601 格式） |
| `agent_id` | `Option<String>` | 按 Agent ID 筛选 |
| `provider_id` | `Option<String>` | 按 Provider ID 筛选 |
| `model_id` | `Option<String>` | 按模型 ID 筛选 |

所有字段均为空字符串安全 -- 空字符串等价于 `None`，不会生成 WHERE 子句。

### build_session_filter

用于 session/message 关联查询，签名：

```rust
fn build_session_filter(
    filter: &DashboardFilter,
    session_alias: &str,       // 表别名，通常 "s"
    message_alias: Option<&str>, // 如有 JOIN messages 则传 "m"
) -> FilterClause
```

自动注入的硬编码条件：
- `{session_alias}.is_cron = 0` -- 排除定时任务会话
- `{session_alias}.parent_session_id IS NULL` -- 排除子 Agent 会话

时间范围过滤逻辑：
- 当提供 `message_alias` 时，时间条件作用于 `{message_alias}.timestamp`
- 否则作用于 `{session_alias}.created_at`

### build_log_filter

用于 LogDB 查询，仅支持 `start_date`、`end_date`、`agent_id` 三个维度（日志表无 provider/model 字段）。

### params_ref 辅助函数

将 `Vec<Box<dyn ToSql>>` 转换为 `Vec<&dyn ToSql>`，适配 rusqlite 的参数绑定 API。

## 聚合查询维度（7 个）

### 1. Overview 概览

**函数**：`query_overview(session_db, log_db, cron_db, filter) -> OverviewStats`

**数据源**：SessionDB + CronDB（跨库查询）

**返回字段**：

| 字段 | 类型 | 说明 |
|------|------|------|
| `total_sessions` | `u64` | 会话总数 |
| `total_messages` | `u64` | 消息总数 |
| `total_input_tokens` | `u64` | 输入 token 总量 |
| `total_output_tokens` | `u64` | 输出 token 总量 |
| `total_tool_calls` | `u64` | 工具调用总次数 |
| `total_errors` | `u64` | 错误消息总数 |
| `active_agents` | `u64` | 活跃 Agent 数（DISTINCT agent_id） |
| `active_cron_jobs` | `u64` | 活跃定时任务数 |
| `estimated_cost_usd` | `f64` | 估算总成本（按模型分组计算后汇总） |
| `avg_ttft_ms` | `Option<f64>` | 平均首 Token 响应时间 |

**实现要点**：成本估算通过 `GROUP BY s.model_id` 按模型分组计算后求和，而非用总 token 数一次性估算，确保多模型场景下定价准确。

### 2. Token 用量趋势

**函数**：`query_token_usage(session_db, filter) -> DashboardTokenData`

**返回结构**：

- `trend: Vec<TokenUsageTrend>` -- 按天聚合
  - `date` / `input_tokens` / `output_tokens` / `avg_ttft_ms`
- `by_model: Vec<TokenByModel>` -- 按模型分组，按总 token 降序
  - `model_id` / `provider_name` / `input_tokens` / `output_tokens` / `estimated_cost_usd` / `avg_ttft_ms`
- `total_cost_usd: f64` -- 所有模型成本之和

### 3. 工具使用统计

**函数**：`query_tool_usage(session_db, filter) -> Vec<ToolUsageStats>`

按 `tool_name` 分组，按调用次数降序排列：

| 字段 | 说明 |
|------|------|
| `tool_name` | 工具名称 |
| `call_count` | 调用次数 |
| `error_count` | 错误次数 |
| `avg_duration_ms` | 平均耗时（毫秒） |
| `total_duration_ms` | 总耗时（毫秒） |

过滤条件额外添加 `tool_name IS NOT NULL AND tool_name != ''`。

### 4. 会话趋势

**函数**：`query_sessions(session_db, filter) -> DashboardSessionData`

**返回结构**：

- `trend: Vec<SessionTrend>` -- 按天聚合
  - `date` / `session_count`（DISTINCT s.id）/ `message_count`
- `by_agent: Vec<SessionByAgent>` -- 按 Agent 分组，按会话数降序
  - `agent_id` / `session_count` / `message_count` / `total_tokens`

### 5. 错误趋势

**函数**：`query_errors(log_db, filter) -> DashboardErrorData`

**数据源**：LogDB（非 SessionDB）

**返回结构**：

- `trend: Vec<ErrorTrend>` -- 按天聚合 error/warn 数量
- `by_category: Vec<ErrorByCategory>` -- 仅 error 级别，按 category 分组降序
- `total_errors: u64` / `total_warnings: u64`

### 6. 任务统计

**函数**：`query_tasks(session_db, cron_db, filter) -> DashboardTaskData`

**数据源**：SessionDB（subagent_runs 表）+ CronDB（cron_jobs + cron_run_logs 表）

**Cron 统计** (`CronJobStats`)：

| 字段 | 说明 |
|------|------|
| `total_jobs` / `active_jobs` | 任务总数 / 活跃任务数 |
| `total_runs` / `success_runs` / `failed_runs` | 运行总次数及成功/失败分布 |
| `avg_duration_ms` | 平均执行耗时 |

**子 Agent 统计** (`SubagentStats`)：

| 字段 | 说明 |
|------|------|
| `total_runs` / `completed` / `failed` / `killed` | 运行次数及状态分布 |
| `total_input_tokens` / `total_output_tokens` | Token 消耗 |
| `avg_duration_ms` | 平均执行耗时 |

### 7. 系统指标

**函数**：`query_system_metrics() -> SystemMetrics`

**数据源**：sysinfo crate（进程级采集，非数据库查询）

**采集流程**：两次 `refresh_processes_specifics` 间隔 200ms 以获取准确的 CPU 使用率增量。

**返回字段**：

| 字段 | 说明 |
|------|------|
| `process_cpu_percent` | 进程 CPU 使用率（多核可超 100%） |
| `cpu_count` | CPU 核心数 |
| `memory.rss_bytes` | 常驻内存（RSS） |
| `memory.virtual_bytes` | 虚拟内存 |
| `memory.system_total_bytes` | 系统总内存 |
| `memory.rss_percent` | RSS 占系统总内存百分比 |
| `disk_io.read_bytes` / `written_bytes` | 进程磁盘读写总量 |
| `process_uptime_secs` | 进程运行时间 |
| `pid` / `os_name` / `host_name` | 进程与系统信息 |
| `system_uptime_secs` | 系统运行时间 |

## 详情查询（5 个）

| 函数 | 返回类型 | 数据源 | 排序 | 限制 |
|------|----------|--------|------|------|
| `query_session_list` | `Vec<DashboardSessionItem>` | SessionDB | `updated_at DESC` | 100 |
| `query_message_list` | `Vec<DashboardMessageItem>` | SessionDB | `timestamp DESC` | 100 |
| `query_tool_call_list` | `Vec<DashboardToolCallItem>` | SessionDB | `timestamp DESC` | 100 |
| `query_error_list` | `Vec<DashboardErrorItem>` | LogDB | `timestamp DESC` | 100 |
| `query_agent_list` | `Vec<DashboardAgentItem>` | SessionDB | `sess_count DESC` | 无限制 |

**共同特征**：
- 所有详情查询均支持完整的 `DashboardFilter` 筛选
- 除 `query_agent_list` 外均有 `LIMIT 100` 硬编码限制
- `query_message_list` 的 content 字段通过 `SUBSTR(m.content, 1, 200)` 截取前 200 字符预览
- `query_error_list` 仅返回 `level IN ('error', 'warn')` 的日志条目

## 成本估算引擎

### 计算流程

```mermaid
flowchart TD
    A[输入: model_id, input_tokens, output_tokens] --> B{匹配模型定价}
    B -->|命中| C[获取 input_price, output_price<br/>单位: USD / 1M tokens]
    B -->|未命中| D[使用默认定价<br/>input: $3 / output: $15]
    C --> E[计算成本]
    D --> E
    E --> F["cost = (input_tokens * input_price<br/>+ output_tokens * output_price)<br/>/ 1,000,000"]
    F --> G[返回 f64 USD]
```

### 模型定价表

匹配规则使用 `model_id.contains()` 子串匹配，按优先级从上到下首次命中。

| 厂商 | 模型 | Input ($/1M) | Output ($/1M) |
|------|------|-------------|---------------|
| **Anthropic** | claude-3-5-sonnet / claude-3.5-sonnet | 3.00 | 15.00 |
| | claude-3-5-haiku / claude-3.5-haiku | 0.80 | 4.00 |
| | claude-3-opus / claude-3.0-opus | 15.00 | 75.00 |
| | claude-3-sonnet | 3.00 | 15.00 |
| | claude-3-haiku / claude-haiku-3 | 0.25 | 1.25 |
| | claude-4 / claude-sonnet-4 | 3.00 | 15.00 |
| | claude-opus-4 | 15.00 | 75.00 |
| **OpenAI** | gpt-4o-mini | 0.15 | 0.60 |
| | gpt-4o | 2.50 | 10.00 |
| | gpt-4-turbo | 10.00 | 30.00 |
| | gpt-4 | 30.00 | 60.00 |
| | gpt-3.5 | 0.50 | 1.50 |
| | o1-mini | 3.00 | 12.00 |
| | o1 | 15.00 | 60.00 |
| | o4-mini | 1.10 | 4.40 |
| | o3-mini | 1.10 | 4.40 |
| | o3 | 10.00 | 40.00 |
| **Google** | gemini-2.5-pro | 1.25 | 10.00 |
| | gemini-2.5-flash | 0.15 | 0.60 |
| | gemini-2.0-flash | 0.10 | 0.40 |
| | gemini-1.5-pro | 1.25 | 5.00 |
| | gemini-1.5-flash | 0.075 | 0.30 |
| **xAI** | grok-4-fast / grok-4-1-fast | 0.20 | 0.50 |
| | grok-4.20 | 2.00 | 6.00 |
| | grok-4 | 3.00 | 15.00 |
| | grok-3-mini | 0.30 | 0.50 |
| | grok-3-fast | 5.00 | 25.00 |
| | grok-3 | 3.00 | 15.00 |
| | grok-code | 0.20 | 1.50 |
| **Mistral** | codestral | 0.30 | 0.90 |
| | devstral | 0.40 | 2.00 |
| | magistral | 0.50 | 1.50 |
| | pixtral | 2.00 | 6.00 |
| | mistral-large | 0.50 | 1.50 |
| | mistral-medium | 0.40 | 2.00 |
| | mistral-small | 0.10 | 0.30 |
| **DeepSeek** | deepseek-reasoner / DeepSeek-R1 | 0.55 | 2.19 |
| | deepseek / DeepSeek | 0.27 | 1.10 |
| **Qwen** | qwen-max / qwen3-max | 2.40 | 9.60 |
| | qwen-plus / qwq-plus | 0.80 | 2.00 |
| | qwen-turbo | 0.30 | 0.60 |
| | qwen (通配) | 0.30 | 0.60 |
| **Zhipu (GLM)** | glm-5-turbo | 1.20 | 4.00 |
| | glm-5 | 1.00 | 3.20 |
| | glm-4.7-flash | 0.07 | 0.40 |
| | glm-4.7 / glm-4-7 | 0.60 | 2.20 |
| | glm-4.6v | 0.30 | 0.90 |
| | glm-4.6 | 0.60 | 2.20 |
| | glm-4.5-flash | 0.00 | 0.00 |
| | glm-4.5 | 0.60 | 2.20 |
| **MiniMax** | MiniMax / minimax | 0.30 | 1.20 |
| **Meta** | Llama-4-Maverick | 0.27 | 0.85 |
| | Llama-4-Scout | 0.18 | 0.59 |
| | Llama-3.3-70B / llama-3.3-70b | 0.88 | 0.88 |
| **Groq** | mixtral | 0.24 | 0.24 |
| **(默认)** | 未匹配模型 | 3.00 | 15.00 |

## 查询流程

```mermaid
sequenceDiagram
    participant FE as 前端 (React)
    participant CMD as Tauri 命令层
    participant Q as queries.rs / detail_queries.rs
    participant F as filters.rs
    participant C as cost.rs
    participant DB as SQLite (SessionDB / LogDB / CronDB)

    FE->>CMD: invoke("dashboard_overview", { filter })
    CMD->>Q: query_overview(session_db, log_db, cron_db, &filter)
    Q->>F: build_session_filter(&filter, "s", Some("m"))
    F-->>Q: FilterClause { where_sql, params }
    Q->>DB: SELECT COUNT(*), SUM(tokens_in), ...
    DB-->>Q: 原始行数据
    Q->>DB: SELECT model_id, SUM(tokens_in), SUM(tokens_out) GROUP BY model_id
    DB-->>Q: 按模型分组的 token 数据
    loop 每个模型
        Q->>C: estimate_cost(model_id, input, output)
        C-->>Q: cost_usd: f64
    end
    Q-->>CMD: OverviewStats { ... }
    CMD-->>FE: JSON (camelCase 序列化)
```

## 图表数据格式

前端通过 `invoke()` 获取的 JSON 数据遵循 camelCase 命名（`#[serde(rename_all = "camelCase")]`）。

### 趋势图数据（折线图 / 面积图）

```json
{
  "trend": [
    { "date": "2026-04-01", "inputTokens": 150000, "outputTokens": 45000, "avgTtftMs": 320.5 },
    { "date": "2026-04-02", "inputTokens": 180000, "outputTokens": 52000, "avgTtftMs": 295.1 }
  ]
}
```

### 分组数据（柱状图 / 饼图）

```json
{
  "byModel": [
    { "modelId": "claude-sonnet-4", "providerName": "anthropic", "inputTokens": 500000, "outputTokens": 150000, "estimatedCostUsd": 3.75, "avgTtftMs": 310.2 }
  ]
}
```

### 概览卡片数据

```json
{
  "totalSessions": 42,
  "totalMessages": 1280,
  "totalInputTokens": 2500000,
  "totalOutputTokens": 750000,
  "totalToolCalls": 890,
  "totalErrors": 12,
  "activeAgents": 3,
  "activeCronJobs": 5,
  "estimatedCostUsd": 12.35,
  "avgTtftMs": 305.7
}
```

## 关键源文件

| 文件 | 职责 |
|------|------|
| `crates/oc-core/src/dashboard/mod.rs` | 模块入口，re-export 公开 API |
| `crates/oc-core/src/dashboard/types.rs` | 20 个数据结构（Filter + Stats + Detail Items + SystemMetrics） |
| `crates/oc-core/src/dashboard/filters.rs` | build_session_filter / build_log_filter 筛选器构建 |
| `crates/oc-core/src/dashboard/queries.rs` | 7 个聚合查询（overview / token / tool / session / error / task / system） |
| `crates/oc-core/src/dashboard/detail_queries.rs` | 5 个详情列表查询（session / message / tool_call / error / agent） |
| `crates/oc-core/src/dashboard/cost.rs` | 模型定价表与成本计算公式 |
| `src-tauri/src/commands/dashboard.rs` | - | Tauri 命令注册层（invoke 入口） |
| `src/components/dashboard/` | - | 前端 recharts 图表组件 |
