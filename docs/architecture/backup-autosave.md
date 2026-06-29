# 备份 / 自动快照（Backup / Autosave）

> 返回 [文档索引](../README.md) | 主源码：[`crates/ha-core/src/backup.rs`](../../crates/ha-core/src/backup.rs)；调用方分散在 [`config/persistence.rs`](../../crates/ha-core/src/config/persistence.rs)、[`user_config.rs`](../../crates/ha-core/src/user_config.rs)、[`guardian.rs`](../../crates/ha-core/src/guardian.rs)、[`self_diagnosis.rs`](../../crates/ha-core/src/self_diagnosis.rs)

## 概述

Backup / Autosave 是**配置安全网**：它不持有任何业务数据，只在配置写盘的关键路径上自动留痕，让"一次手滑改坏配置"或"崩溃循环导致配置损坏"都能回滚。子系统由两套**预算独立、入口独立、用途不同**的备份机制组成：

- **配置 autosave（细粒度）**：每次 `config.json` / `user.json` 写盘**前**，自动把**旧文件**复制成一个单文件快照，文件名编码"这次写的原因"。保留**最近 50 份**，专为"撤销某次设置编辑"服务。
- **全量崩溃备份（粗粒度）**：崩溃恢复诊断命中阈值时（或用户手动触发），把**整套配置目录**（`config.json` + `user.json` + `credentials/auth.json` + `agents/`）打包进一个时间戳目录。保留**最近 5 份**，专为"崩溃自愈 / 整体回滚"服务。

两套机制全部落在 `~/.hope-agent/` 下、全部以"失败永不阻塞合法写"为铁律实现，所有逻辑集中在 ha-core 的 `backup.rs`（零 Tauri 依赖），桌面 / server 只做薄壳转发。

> **勿混淆**：self-update 子系统另有一个完全独立的 [`updater/backup.rs`](../../crates/ha-core/src/updater/backup.rs)，负责 bare-binary 的 store / prune / restore，与本文的配置备份**毫无关系**。见 [自升级](self-update.md)。

## 子系统定位与两套备份分工

| 维度 | 全量崩溃备份 | 配置 autosave |
|---|---|---|
| 入口 | `create_backup` | `snapshot_before_write` |
| 触发 | 崩溃恢复诊断命中阈值 / 用户手动 | `config.json` / `user.json` 写盘前——主路径 `mutate_config` / `save_user_config_to_disk`，旁路 `guardian` / `onboarding` 等直接 `save_config + scope_save_reason` 同样触发 |
| 范围 | `config.json` + `user.json` + `credentials/auth.json` + `agents/`（整目录递归） | 单文件（`config.json` **或** `user.json`） |
| 粒度 | 整套配置目录快照 | 单次写盘前的旧文件 |
| 落盘 | `backups/backup_{时间戳}/`（目录） | `backups/autosave/{...}.json`（单文件） |
| 保留 | `MAX_BACKUPS = 5` | `MAX_AUTOSAVES = 50` |
| 列举 / 恢复 | `list_backups` / `restore_backup` | `list_autosaves` / `restore_autosave` |
| 用途 | 崩溃自愈、整体回滚 | 撤销某一次设置编辑 |

**预算分离是有意设计**：若两套共用配额，一阵密集的设置编辑产生的 autosave 洪水会把最后一次用户手动全量备份挤掉。`auto_fix` 在 `config.json` 损坏时恢复用的是**崩溃备份目录**（`list_backups().first()` 取最新），**不是** autosave——这条红线务必牢记。

## 核心数据结构

全部定义在 `backup.rs`：

| 符号 | 角色 |
|---|---|
| `BackupInfo` | 全量备份条目，序列化 camelCase，`{ name, path, created_at: u64 }`；`created_at` 取自目录 `metadata.created()` |
| `AutosaveEntry` | autosave 条目，序列化 camelCase，`{ id, timestamp, kind, category, source }`；`id` 是文件名 stem，其余四段从文件名 `splitn(4, "__")` 解析 |
| `SaveReason` | 私有 struct，`{ category, source }` 两字段，描述"下一次 save 的原因" |
| `SaveReasonGuard` | RAII guard，`Drop` 时清空 thread-local `NEXT_SAVE_REASON`——即使本次 save 未真正发生也不污染后续写 |
| `MAX_BACKUPS` | `const usize = 5`，全量崩溃备份保留数 |
| `MAX_AUTOSAVES` | `const usize = 50`，autosave 保留数 |

## 数据流

### 全量崩溃备份：create_backup → restore_backup

- **`create_backup`**：把 `config.json` / `user.json` / `credentials/auth.json` 逐文件复制、`agents/` 经 `copy_dir_recursive` 整目录复制到新建的 `backups/backup_{UTC 时间戳}/`，时间戳格式 `%Y-%m-%dT%H-%M-%S`。**单文件 copy 失败只 `app_warn` 继续**，不中断整次备份。末尾调 `rotate_backups_internal` 轮转，返回备份目录路径字符串。
- **`list_backups`**：扫 `backups_dir` 下 `backup_` 前缀目录，按目录名**倒序**（最新优先）返回 `BackupInfo` 列表。
- **`restore_backup`**：按名定位备份目录，把其中文件复制回 `root`——`agents/` **先删后复制**（保证恢复结果是备份时的精确快照而非合并）。末尾调 [`config::reload_cache_from_disk`](config-system.md) 刷新内存配置快照，让恢复立即对运行中实例生效。

### 配置 autosave：snapshot_before_write

`snapshot_before_write(src, kind)` 是配置 autosave 的**唯一入口**（`kind ∈ "config" | "user"`），步骤：

1. 经 `take_save_reason` 取出并清空 thread-local reason 标签（缺省 `category="unknown"` / `source="unknown"`）。
2. 若 `src` 文件**不存在**或读取出错 → 早退，但**仍消费掉 reason**（防止标签泄漏给下一次无关写）。
3. `src` 存在 → 把它复制进 `autosave_dir`，文件名编码 `{timestamp}__{kind}__{category}__{source}.json`，末尾 `rotate_autosaves` 轮转。
4. **过程中任何错误只 `app_warn`，绝不向上传播**——合法写永远优先于快照成功。

### reason 标签机制（thread-local）

autosave 文件名里的 `category` / `source` 来自一个 thread-local 标签，生命周期由 RAII guard 管理：

- **`scope_save_reason(category, source)`**：设置 thread-local `NEXT_SAVE_REASON` 并返回 `SaveReasonGuard`。调用方**必须持有 guard 直到 save 完成**——guard `Drop` 时清空标签，保证即使 save 没发生也不污染后续写。
- **`take_save_reason`**：取出并清空标签（**消费一次**语义）；缺省返回 `unknown/unknown`。

典型用法：写入路径先 `let _guard = scope_save_reason(category, source);`，随后 `load → mutate → save`，save 内部对旧文件调 `snapshot_before_write`，由它 `take_save_reason` 拿到标签写进文件名。

### autosave 列举与回滚

- **`list_autosaves`**：扫 `autosave_dir` 下 `.json`，按 `__` 解析四段（**非四段直接跳过**），按 `timestamp` 倒序（最新优先）。
- **`restore_autosave(id)`**：按 `id`（文件名 stem）定位快照，按 `kind` 解析目标路径（`config` → config_path / `user` → user_config_path）。**回滚前先对当前态自快照**（reason 标 `rollback-to:…`，保证回滚本身可逆），再覆盖目标文件：
  - `config` 走 `reload_cache` 刷新内存 + emit `config:changed`（`category = "__rollback__"`）。
  - `user` 走 emit `config:changed`（`category = "user"`）。

## 保留策略与轮转

两套各自独立轮转，**均依赖文件名 / 目录名字典序 == 时间序**（时间戳前缀）：

- **`rotate_backups_internal(keep)`**：按目录名升序排列全量备份，超 `keep` 删最旧。
- **`rotate_autosaves(keep)`**：按文件名升序排列 autosave `.json`，超 `keep` 删最旧。

辅助函数：

- **`sanitize_slug`**：把 `category` / `source` 段中非 `[A-Za-z0-9_-]` 的字符替换为 `-`，保证文件名安全。
- **`copy_dir_recursive`**：递归复制目录，供 `agents/` 的备份与恢复使用。

> 改时间戳格式会破坏"字典序 == 时间序"的前提，进而破坏轮转对"最旧"的判定——属红线。

## 持久化路径

集中由 [`paths.rs`](../../crates/ha-core/src/paths.rs) 提供（`backups_dir()` / `autosave_dir()`）：

| 路径 | 内容 |
|---|---|
| `~/.hope-agent/backups/` | 全量崩溃备份根 |
| `~/.hope-agent/backups/backup_{UTC %Y-%m-%dT%H-%M-%S}/` | 单次全量备份目录（含 `config.json` / `user.json` / `credentials/auth.json` / `agents/` 递归副本） |
| `~/.hope-agent/backups/autosave/` | 配置 autosave 根 |
| `~/.hope-agent/backups/autosave/{%Y-%m-%dT%H-%M-%S-%3f}__{kind}__{category}__{source}.json` | 单个 autosave 快照——**元数据全编码进文件名，无 sidecar 索引** |

autosave 文件名带毫秒（`%3f`），避免同一秒内多次写盘碰撞；全量备份目录按秒命名（不会高频触发故无需毫秒）。

## 调用方与集成点

| 调用方 | 行为 |
|---|---|
| `config::mutate_config`（[`config/persistence.rs`](../../crates/ha-core/src/config/persistence.rs)） | **所有 `AppConfig` 写入的唯一入口**：取写锁 + `scope_save_reason(reason)` + `load → mutate → save`，save 内部对旧 `config.json` 调 `snapshot_before_write`。是 autosave 标签的主要来源 |
| `user_config::save_user_config_to_disk` | `user.json` 写盘前调 `snapshot_before_write(path, "user")` |
| `guardian::set_enabled_in_config`（[`guardian.rs`](../../crates/ha-core/src/guardian.rs)） | `guardian.enabled` 是 `AppConfig` schema **之外**的 raw JSON 字段，刻意绕过 `mutate_config` 直接读写 raw JSON；但写前仍**手动** `scope_save_reason("guardian", "guardian")` + `snapshot_before_write`，守住 rollback 契约 |
| `guardian::run_recovery` | 崩溃恢复中 `crash_count` 命中诊断阈值时调 `create_backup()` 做全量备份，并 `crash_journal.set_last_backup` 记录 |
| `self_diagnosis::try_restore_config_from_backup`（[`self_diagnosis.rs`](../../crates/ha-core/src/self_diagnosis.rs)） | `config.json` 损坏时取 `list_backups().first()`（**最新全量备份**）经 `restore_backup` 恢复——**不用 autosave** |
| onboarding / 崩溃恢复 | 经上述 guardian / self_diagnosis 路径间接触发全量备份 |

## 对外接口面（Tauri / HTTP）

桌面与 server 各暴露两组命令，分别对应全量崩溃备份（`crash/backups`）与配置 autosave（`settings/backups`）：

| 用途 | Tauri 命令 | HTTP 路由 |
|---|---|---|
| 列出全量备份 | `list_backups_cmd` | `GET /api/crash/backups` |
| 创建全量备份 | `create_backup_cmd` | `POST /api/crash/backups` |
| 恢复全量备份 | `restore_backup_cmd` | `POST /api/crash/backups/restore` |
| 列出 autosave | `list_settings_backups_cmd` | `GET /api/settings/backups` |
| 恢复 autosave | `restore_settings_backup_cmd` | `POST /api/settings/backups/restore` |

`list_settings_backups` / `restore_settings_backup` 同时以**工具**形式提供给模型（`ToolTier::Standard`、`internal`，`default_for_main: true` / `default_for_others: false` / `default_deferred: true`——主 Agent 默认加载、其它 Agent 不加载，开启延迟加载模式时是 `tool_search` 可发现的 deferred 候选），见 [工具系统](tool-system.md)。命令对照详见 [api-reference.md](api-reference.md)。

## 事件

| 事件 | 触发 |
|---|---|
| `config:changed` | `restore_autosave` 回滚后发出（`config` 回滚 `category = "__rollback__"`，`user` 回滚 `category = "user"`）；`restore_backup` 经 `reload_cache_from_disk` 刷新内存快照 |

## 安全 / 红线

- **两套备份勿混淆**：全量崩溃备份（`create_backup`，崩溃阈值命中触发一次，快照 config + user + credentials + agents，保留 5）与配置 autosave（`snapshot_before_write`，每次配置写盘前触发，单文件细粒度，保留 50）用途与预算都不同。`auto_fix` 恢复 config 用的是**崩溃备份目录**（`list_backups().first()`），不是 autosave。
- **失败永不阻塞合法写**：`snapshot_before_write` 内部所有错误只 `app_warn` 不向上传播；`create_backup` 内单文件 copy 失败也只 warn 继续。备份是安全网，绝不能因为安全网破损而拦住用户的正常配置写。
- **reason 标签消费语义**：`scope_save_reason` 返回的 guard 必须持有到 save 完成；`take_save_reason` 取出即清空（消费一次）。`snapshot_before_write` 在 `src` 不存在 / 出错的早退路径也会消费掉 reason，防止泄漏给下一次无关写。
- **文件名结构是稳定契约**：autosave 文件名靠 `splitn(4, "__")` 解析，分隔符 `__` 不可改；`category` / `source` 经 `sanitize_slug` 兜底安全字符，但四段结构本身是契约。
- **回滚前自快照不可删**：`restore_autosave` 在覆盖前先对当前态自快照（reason `rollback-to:`），保证"回滚这个动作本身"也可逆——这一步是闭环可逆性的关键，不要省略。
- **guardian.enabled 刻意绕过 mutate_config**：它是 `AppConfig` schema 之外的 raw JSON 字段，故意直接读写，但写前仍手动 `scope_save_reason` + `snapshot_before_write` 守 rollback 契约——**不要把它塞回 `mutate_config`**。
- **预算分离有意为之**：`MAX_BACKUPS` / `MAX_AUTOSAVES` 各算各的，防一阵设置编辑的 autosave 洪水把最后一次用户手动全量备份挤掉。
- **轮转依赖时间序前缀**：轮转判"最旧"靠文件名 / 目录名字典序 == 时间序（时间戳前缀），改时间戳格式会破坏这一前提。
- **与 updater 备份隔离**：[`updater/backup.rs`](../../crates/ha-core/src/updater/backup.rs)（self-update 的 bare-binary store / most_recent / prune / restore）是另一个完全独立的模块，与本配置备份子系统无关，勿混淆。

## 与相邻子系统的关系

| 子系统 | 关系 |
|---|---|
| [配置系统](config-system.md) | `mutate_config` 写盘前经 `snapshot_before_write` 落旧文件；`scope_save_reason` 提供 `(category, source)` 人类可读标签；回滚经 `reload_cache_from_disk` + `config:changed` 生效 |
| [可靠性 / 崩溃恢复](reliability.md) | `guardian::run_recovery` 崩溃阈值命中调 `create_backup`；`self_diagnosis::try_restore_config_from_backup` 用最新全量备份自愈损坏 config |
| [工具系统](tool-system.md) | `list_settings_backups` / `restore_settings_backup` 作 `Standard` tier `internal` 工具，主 Agent 默认加载（`default_deferred: true`，延迟加载模式下为 `tool_search` 可发现的 deferred 候选） |
| [自升级](self-update.md) | 独立的 `updater/backup.rs` 负责 binary 备份，与本子系统无关 |
| `ha-settings` 技能 | SKILL.md 登记 autosave 自动快照说明 + 两个 settings-backup 工具用法 + "Rollback is built-in" 指引 |

## 关键文件索引

| 文件 | 角色 |
|---|---|
| [`crates/ha-core/src/backup.rs`](../../crates/ha-core/src/backup.rs) | 子系统全部逻辑：`create_backup` / `restore_backup` / `snapshot_before_write` / `scope_save_reason` / `list_*` / `restore_autosave` / 轮转 / `sanitize_slug` + 两个 `MAX_*` 常量 |
| [`crates/ha-core/src/config/persistence.rs`](../../crates/ha-core/src/config/persistence.rs) | `mutate_config` —— autosave 标签主来源、`AppConfig` 写唯一入口 |
| [`crates/ha-core/src/user_config.rs`](../../crates/ha-core/src/user_config.rs) | `save_user_config_to_disk` —— `user.json` 写前快照 |
| [`crates/ha-core/src/guardian.rs`](../../crates/ha-core/src/guardian.rs) | `set_enabled_in_config` raw-JSON 旁路守 rollback 契约 + `run_recovery` 崩溃备份集成 |
| [`crates/ha-core/src/self_diagnosis.rs`](../../crates/ha-core/src/self_diagnosis.rs) | `try_restore_config_from_backup` 损坏 config 自愈 |
| [`crates/ha-core/src/paths.rs`](../../crates/ha-core/src/paths.rs) | `backups_dir()` / `autosave_dir()` 路径单一来源 |
