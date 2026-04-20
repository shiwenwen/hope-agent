# 配置系统

应用配置（`AppConfig`，持久化到 `~/.hope-agent/config.json`）的读写 contract。所有面向用户的开关、Provider 列表、工具配置、UI 偏好等都住在这一个结构里。

## 核心原则

**单一真相源**：整个进程只有一份内存中的 `AppConfig`，由 [`ha_core::config::cached_config()`](../../crates/ha-core/src/config/persistence.rs) 发布为 `ArcSwap<AppConfig>`。

- **读**一律走 `cached_config()` → 返回 `Arc<AppConfig>`。lock-free，单次 atomic acquire load + Arc refcount bump，开销 O(ns)，hot path 零压力。
- **写**一律走 `mutate_config((category, source), |cfg| { ... })`。进程级全局 `Mutex<()>` 串行化"读最新快照 → 应用 mutation → 持久化到磁盘 → 原子 swap 回 ArcSwap → emit `config:changed` 事件"。

读写严格异步：读路径永不等待写路径（ArcSwap 无锁），写路径之间互斥（同一时刻只能有一个 `mutate_config` 运行）。

## API 形态

```rust
use ha_core::config::{cached_config, mutate_config};

// 读
let config = cached_config();             // Arc<AppConfig>
if config.canvas.enabled { /* ... */ }

// 写
mutate_config(("canvas", "settings-ui"), |cfg| {
    cfg.canvas.enabled = true;
    Ok(())
})?;
```

`mutate_config` 的 FnOnce 闭包返回 `anyhow::Result<T>`，错误从闭包透传出来（校验失败可以在闭包内直接 `Err(...)`，不落盘）。返回的 `T` 可以把 mutation 过程中算出的派生数据（比如新建的 ID）传回调用方。

`reason: (&str, &str)` 是 `(category, source)` 的二元组：
- `category` ——改的是哪个子系统（"image_generate" / "memory_budget" / "shortcuts" / "security.ssrf" / ...）
- `source` ——从哪触发的（"settings-ui" / "http" / "oauth-finalize" / "cron" / ...）

两个字段会：
1. 作为 autosave 备份文件的 tag 写入 `~/.hope-agent/backups/autosave/`，方便后续 **Settings → Backups → Rollback** 面板识别"是谁在什么时候改的什么"。
2. （未来）作为 `config:changed` 事件的 payload，让前端知道只需要刷新哪些面板。

## 为什么不是简单的 `Mutex<AppConfig>`

历史上 `AppState::config: Mutex<AppConfig>` 就是这么写的，叠加后来引入的 `cached_config()` ArcSwap 导致**两份真相源**：

| 读写路径 | 读哪份 | 写哪份 |
|---|---|---|
| Tauri `chat.rs` 等 command | `state.config.lock().await` (Mutex) | （不写） |
| `save_*_config` Tauri 命令 | `load_config()` 克隆 cached_config | `save_config` 只刷新 cached_config |
| `save_*_config` HTTP 路由 | `load_config()` 克隆 cached_config | `save_config` 只刷新 cached_config |
| ha-core 内部（tools / context / agent）| `cached_config()` | 不写 |

结果：用户在 UI 点保存，cached_config 更新了、磁盘更新了，但 `AppState::config` 那个 Mutex 里还是 App 启动时的快照——chat.rs 读的正是这个 Mutex，于是工具可用性 / 温度 / 提供商等等全部 stale 到下次 App 启动。最典型的现象是 UI 里启用 Gemini 图像生成后，`image_generate` 工具永远不被注入到 LLM 的 `tools[]`，模型在 thinking 里直接说 "tool list mentions image_generate but it isn't in the functions list"——看起来像幻觉，实际是客户端真的没把它发出去。

解决方案就是**彻底移除**那份 `Mutex<AppConfig>` 字段，把整个进程收敛到 `cached_config`（读）+ `mutate_config`（写）一套 contract。

## 并发模型

```
                     读                    写
                   ┌──┐                 ┌──────┐
┌─────────────┐    │  ▼                 ▼      │
│  Frontend   │    │  Arc<AppConfig>    │      │
│  / IM /     │    │  （ArcSwap）       │      │
│  ha-core    │◄───┘                    │      │
└─────────────┘                         │      │
                                        ▼      │
                                ┌──────────────┴─┐
                                │ mutate_config  │
                                │ (全局 Mutex<()>) │
                                │ ┌────────────┐ │
                                │ │ 1. 克隆快照 │ │
                                │ │ 2. FnOnce  │ │
                                │ │ 3. 写磁盘   │ │
                                │ │ 4. swap    │ │
                                │ │ 5. emit    │ │
                                │ └────────────┘ │
                                └────────────────┘
```

- 读路径**从不阻塞**：ArcSwap 允许无限个并发 reader。
- 写路径互斥：全局 `std::sync::Mutex<()>` 确保 critical section 串行化（避免两个请求同时"读-改-写"时后写覆盖先写的 lost-update 场景）。
- 写路径持锁时间 = 一次 AppConfig `clone()` + 闭包执行 + `serde_json::to_string_pretty` + 一次 `std::fs::write` + 一次 Arc swap。磁盘 IO 是 blocking 的，但写频率很低（用户点保存），不会阻塞 tokio runtime 的其它 worker。
- 快照的原子性：`cache().store(Arc::new(new))` 是 ArcSwap 的 release store，任何 `cached_config()` 调用要么看到旧快照要么看到新快照，绝不会看到半更新状态。
- `mutate_config` 是同步函数（非 async），在 tokio async context 下调用它持有的 std::sync::Mutex 属于**短时 critical section**，不会卡住 runtime。

## 事件通知

`save_config`（无论是直接调还是通过 `mutate_config`）会在磁盘写入 + ArcSwap 更新完成后，通过 EventBus emit `config:changed` 事件，payload 含 category。前端现有订阅点：

| Hook | 作用 |
|---|---|
| [`src/i18n/i18n.ts`](../../src/i18n/i18n.ts) | 语言切换热更新 |
| [`src/hooks/useTheme.ts`](../../src/hooks/useTheme.ts) | 主题热切换 |
| [`src/hooks/useDangerousModeStatus.ts`](../../src/hooks/useDangerousModeStatus.ts) | Dangerous Mode 开关状态刷新 |
| [`src/lib/notifications.ts`](../../src/lib/notifications.ts) | Notification 设置刷新 |

这些订阅之前就存在，但因为 save 命令从没发出过 `config:changed`（只有 backup restore / settings skill 会发），实际上是死代码。改造后"UI 保存 → 所有订阅者收到通知 → 面板热更新"的链路才真正闭合。

## 与备份 / 回滚联动

`save_config` 写盘前先调用 [`crate::backup::snapshot_before_write`](../../crates/ha-core/src/backup.rs) 把**旧**文件复制到 `~/.hope-agent/backups/autosave/`，文件名含 `(category, source, timestamp)`。用户在设置 → 备份面板可以回滚到任何历史快照。

`mutate_config` 内部调用 `backup::scope_save_reason` 传入 `(category, source)`，作为当次备份的 tag——所以备份面板上能看到 "theme/settings-ui" / "image_generate/settings-ui" / "active_model/slash-channel" 这样的人类可读标签，而不是一排 "unknown/unknown"。

## 迁移历史

| 日期 | 改动 |
|---|---|
| 2026-04-20 | 删除 `AppState::config: Mutex<AppConfig>` 字段；所有 save 路径走 `mutate_config`；所有读路径走 `cached_config()`；`save_config` emit `config:changed`。彻底解决"UI 保存不生效、须重启 App"的 stale 类 bug。 |

## 反面样式

以下写法**不应**再出现：

```rust
// ❌ 不要手动克隆 → 本地改 → 单独调 save_config
let mut store = ha_core::config::load_config()?;
store.canvas.enabled = true;
ha_core::config::save_config(&store)?;   // 没有写锁保护，两个并发请求会丢更新
```

```rust
// ❌ 不要往 AppState 里再塞一份 config 副本
pub struct AppState {
    pub config: Mutex<AppConfig>,   // 历史教训——永远会跟 cached_config 分叉
    ...
}
```

```rust
// ❌ 不要绕过 mutate_config 直接写磁盘
std::fs::write("~/.hope-agent/config.json", ...);   // 破坏 ArcSwap + 备份 + 事件通知
```

新增任何 save 命令时，模板永远是：

```rust
ha_core::config::mutate_config(("<category>", "<source>"), |store| {
    store.<field> = new_value;
    Ok(())
})
.map_err(|e| e.to_string())
```
