# Channel 入站事件类型扩展 — `InboundEvent` 设计与实施

> 目标：把 channel 入站通道从单一的 `MsgContext`（消息）拓宽到 `InboundEvent` 枚举，让 reactions / 消息编辑 / 撤回 / 加退群等非消息事件能统一进入 dispatcher，给上层 agent / IM 渠道一个完整的事件视图。
>
> 这是为飞书全面对齐做的第一步基建（见 [Phase A 协议层补完](../../crates/ha-core/src/channel/feishu/) 已合并），但成果跨所有 12 个 channel，不是飞书专属。

## 0. Context

### 0.1 现状

[`channel/traits.rs:30`](../../crates/ha-core/src/channel/traits.rs#L30) 的 `ChannelPlugin::start_account` 当前签名只允许往 dispatcher 发消息：

```rust
async fn start_account(
    &self,
    account: &ChannelAccountConfig,
    inbound_tx: mpsc::Sender<MsgContext>,   // ← 只有消息
    cancel: CancellationToken,
) -> Result<()>;
```

[`channel/types.rs:231-254`](../../crates/ha-core/src/channel/types.rs#L231) 的 `MsgContext` 是"用户发了一条消息给 bot"的标准化模型，字段全围绕"消息内容":

```rust
pub struct MsgContext {
    pub channel_id, account_id, sender_id, sender_name, sender_username,
    pub chat_id, chat_type, chat_title, thread_id,
    pub message_id, text, media: Vec<InboundMedia>, reply_to_message_id,
    pub timestamp, was_mentioned, raw,
}
```

[`channel/worker/dispatcher.rs:54`](../../crates/ha-core/src/channel/worker/dispatcher.rs#L54) 的消费端也只懂 `mpsc::Receiver<MsgContext>`：

```rust
mut inbound_rx: mpsc::Receiver<MsgContext>,
```

### 0.2 缺口

非消息事件目前**完全没有进入 dispatcher 的通道**，各 channel 各自走旁路或直接丢弃：

| 事件类别 | 现状 |
|---|---|
| 卡片/按钮回调 | 飞书 `card.action.trigger` 走 [`worker/ask_user.rs::try_dispatch_interactive_callback`](../../crates/ha-core/src/channel/worker/ask_user.rs)（旁路，仅给 ask_user 响应用），不经过 dispatcher，也不走 agent |
| 表情回应（reaction） | 全部 channel 都没接 |
| 消息编辑 | 全部 channel 都没接（飞书没有 event；Telegram/Discord 有 `edited_message` event 但也没转发）|
| 消息撤回 | 全部 channel 都没接 |
| 加退群（membership） | 全部 channel 都没接 |
| 已读回执 | 全部 channel 都没接（Telegram 有 `read_receipt`，飞书有 `im.message.message_read_v1`） |

### 0.3 动机

随飞书 Phase A 把协议层连上之后，下一步 hope-agent 想在 IM 渠道里支持：

1. 用户回复 reaction（👍 / 🎉）触发 agent 行动（如"任务确认"）
2. 用户撤回某条消息时，agent 同步从 session history 里删/标记
3. bot 被踢出群时清理对应 channel session（项目反向认领已废弃, Phase A1）
4. 多人群里有新人 join 时，agent 可以选择性发欢迎 / 引导

这些都需要"非消息事件"能从 channel 流到 dispatcher。

### 0.4 非目标

| # | 不做 | 原因 |
|---|---|---|
| N1 | 重写 `MsgContext` 字段语义 | 保持向后兼容；`InboundEvent::Message(MsgContext)` 把现有字段原封封装 |
| N2 | 一次性接齐所有 12 个 channel 的所有事件类型 | 基础设施 PR 只引 enum 不接事件；具体事件分多个后续 PR 按 channel + 事件类型组合按需上 |
| N3 | 给 `ChannelPlugin` trait 加新出站方法（如"发 reaction"） | 此设计只关心入站；出站能力扩展是单独的 trait 演进，不与本计划耦合 |
| N4 | 改前端事件类型 | 当前前端只渲染 message；reaction/recall/membership 渲染在后续 UI PR 处理 |
| N5 | `EditedMessage` 自动改写 session history | 编辑/撤回的"是否落库 / 是否回滚"是 session 子系统决策，本计划只把事件传到位 |

## 1. 影响面盘点

### 1.1 改 `inbound_tx` 类型 — 26 处文件

按当前 `grep -l inbound_tx`，至少触达：

- **统一类型源**：[`channel/types.rs`](../../crates/ha-core/src/channel/types.rs)（新 enum）
- **trait 签名**：[`channel/traits.rs:30-34`](../../crates/ha-core/src/channel/traits.rs#L30)
- **registry**：[`channel/registry.rs`](../../crates/ha-core/src/channel/registry.rs)（创建/分发 channel 给 dispatcher）
- **dispatcher**：[`channel/worker/dispatcher.rs:54`](../../crates/ha-core/src/channel/worker/dispatcher.rs#L54) + 内部 `process_message` 改名/拆分
- **12 个 channel 插件**：每个 `mod.rs` 的 `start_account` + 各 channel 的事件源（gateway / webhook / client / socket / ws_event 等），共 23 个文件:
  - discord/{mod,gateway}.rs
  - feishu/{mod,ws_event}.rs
  - googlechat/{mod,webhook}.rs
  - imessage/{mod,client}.rs
  - irc/{mod,client}.rs
  - line/{mod,webhook}.rs
  - qqbot/{mod,gateway}.rs
  - signal/{mod,client}.rs
  - slack/{mod,socket}.rs
  - telegram/mod.rs
  - wechat/* (按需)
  - whatsapp/* (按需)

### 1.2 旁路调整 — 飞书 card action

[`feishu/ws_event.rs::handle_data_frame`](../../crates/ha-core/src/channel/feishu/ws_event.rs) 当前 `card.action.trigger` 直接调 `try_dispatch_interactive_callback`，不走 dispatcher。本计划**保留这条旁路**——ask_user 的按钮交互不能走 dispatcher（dispatcher 会启 chat round），只能直接回填到 pending question。所以 card action 在 enum 里**不**作为新 variant；它仍然走 ask_user 旁路。

### 1.3 现有 channel 单测

现有有 [`channel/worker/tests.rs`](../../crates/ha-core/src/channel/worker/tests.rs) 的 dispatcher mock 测试，会受 enum 改动影响——所有 mock send 改成 `InboundEvent::Message(...)`。

## 2. 设计

### 2.1 `InboundEvent` 枚举（新增到 `channel/types.rs`）

```rust
/// Top-level event delivered from a channel plugin to the dispatcher.
///
/// `Message` is the canonical payload (a user wrote something for the bot
/// to respond to). All other variants are out-of-band signals — they may
/// or may not trigger an agent round depending on the dispatcher's policy
/// for each variant.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum InboundEvent {
    /// A new user message — full chat round trigger.
    Message(MsgContext),

    /// User added or removed an emoji reaction on an existing message.
    Reaction(ReactionEvent),

    /// User edited the text/content of a previously sent message.
    /// Feishu does not currently expose this; Telegram/Discord do.
    MessageEdited(EditedMessageEvent),

    /// Message was withdrawn by sender. Channel-specific recall windows
    /// (e.g. Feishu 24h, Telegram 48h) determine availability.
    MessageRecalled(RecalledMessageEvent),

    /// Membership change in a chat — user/bot joined or left.
    Membership(MembershipEvent),

    /// User read the bot's last sent message. Spammy on busy chats — the
    /// dispatcher's default policy is to log+drop unless explicitly enabled.
    ReadReceipt(ReadReceiptEvent),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventCommon {
    pub channel_id: ChannelId,
    pub account_id: String,
    pub chat_id: String,
    pub chat_type: ChatType,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Raw platform-specific payload for diagnostics / debugging.
    #[serde(default)]
    pub raw: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReactionEvent {
    #[serde(flatten)]
    pub common: EventCommon,
    pub message_id: String,
    pub sender_id: String,
    pub emoji: String,
    /// `true` = reaction added; `false` = removed.
    pub added: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditedMessageEvent {
    #[serde(flatten)]
    pub common: EventCommon,
    pub message_id: String,
    pub sender_id: String,
    pub new_text: Option<String>,
    pub edited_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecalledMessageEvent {
    #[serde(flatten)]
    pub common: EventCommon,
    pub message_id: String,
    /// Some channels (Telegram) report who recalled; others don't.
    pub recalled_by: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum MembershipAction {
    UserJoined { user_id: String, inviter_id: Option<String> },
    UserLeft   { user_id: String, kicked_by: Option<String> },
    BotJoined  { added_by: Option<String> },
    BotLeft    { removed_by: Option<String> },
    ChatCreated,
    ChatDisbanded,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MembershipEvent {
    #[serde(flatten)]
    pub common: EventCommon,
    #[serde(flatten)]
    pub action: MembershipAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadReceiptEvent {
    #[serde(flatten)]
    pub common: EventCommon,
    pub message_id: String,
    pub reader_id: String,
}
```

### 2.2 trait 改动

```rust
async fn start_account(
    &self,
    account: &ChannelAccountConfig,
-   inbound_tx: mpsc::Sender<MsgContext>,
+   inbound_tx: mpsc::Sender<InboundEvent>,
    cancel: CancellationToken,
) -> Result<()>;
```

`check_access(&MsgContext)` **不变** — 只针对消息类事件做安全决策；其它 variant 在 dispatcher 内单独走"事件级"权限决策（默认放行 + 可配置黑/白名单，见 §3）。

### 2.3 dispatcher 消费端改动

[`channel/worker/dispatcher.rs::spawn_inbound_dispatcher`](../../crates/ha-core/src/channel/worker/dispatcher.rs#L54):

```rust
async fn spawn_inbound_dispatcher(
    mut inbound_rx: mpsc::Receiver<InboundEvent>,
    ...
) {
    while let Some(event) = inbound_rx.recv().await {
        match event {
            InboundEvent::Message(msg) => process_message(msg, ...).await,
            InboundEvent::Reaction(ev) => process_reaction(ev, ...).await,
            InboundEvent::MessageEdited(ev) => process_edited(ev, ...).await,
            InboundEvent::MessageRecalled(ev) => process_recalled(ev, ...).await,
            InboundEvent::Membership(ev) => process_membership(ev, ...).await,
            InboundEvent::ReadReceipt(ev) => process_read_receipt(ev, ...).await,
        }
    }
}
```

**Phase B 第一阶段**（基础设施 PR）的所有非 `Message` 分支都是：

```rust
async fn process_reaction(ev: ReactionEvent, _ctx: ...) {
    app_info!("channel", "inbound", "[{}/{}] reaction {} {} on msg={}",
        ev.common.channel_id, ev.common.account_id,
        if ev.added { "+" } else { "-" }, ev.emoji, ev.message_id);
    // TODO: route to per-event handler (future PR)
}
```

只 log + 不动作。具体事件 → 业务行为是后续 PR 一项一项接（见 §4）。

### 2.4 飞书侧改 — 只是包一层

[`feishu/ws_event.rs::handle_message_event`](../../crates/ha-core/src/channel/feishu/ws_event.rs) 现在末尾：

```rust
inbound_tx.send(msg).await
```

改成：

```rust
inbound_tx.send(InboundEvent::Message(msg)).await
```

其它 11 个 channel 同理改一行。

### 2.5 飞书新增事件解析（Phase B 第二阶段）

参考飞书事件文档，第一批接：

| 飞书 event_type | InboundEvent variant |
|---|---|
| `im.message.reaction.created_v1` | `Reaction { added: true, .. }` |
| `im.message.reaction.deleted_v1` | `Reaction { added: false, .. }` |
| `im.message.recalled_v1` | `MessageRecalled` |
| `im.message.message_read_v1` | `ReadReceipt` |
| `im.chat.member.user.added_v1` | `Membership::UserJoined` |
| `im.chat.member.user.deleted_v1` | `Membership::UserLeft` |
| `im.chat.member.bot.added_v1` | `Membership::BotJoined` |
| `im.chat.member.bot.deleted_v1` | `Membership::BotLeft` |
| `im.chat.created_v1` | `Membership::ChatCreated` |
| `im.chat.disbanded_v1` | `Membership::ChatDisbanded` |

（飞书没有"消息编辑"事件，跳过 `MessageEdited`）

每种事件加一个 `parse_xxx_event` helper + 单测，同 `handle_message_event` 风格。

## 3. 事件级权限策略

`check_access` 只看消息事件。其它事件用 dispatcher 内置的简单策略：

| 事件 | 默认策略 | 配置点 |
|---|---|---|
| Reaction | 落 LearningTracker 埋点 + 不触发 agent round | `event_policies.reaction = "log" \| "respond" \| "drop"` |
| MessageEdited | 同步更新 session messages 表（按 message_id 找原条改 text）+ log；不触发 round | `event_policies.message_edited = "sync" \| "log_only"` |
| MessageRecalled | 同步标记 messages 表 `recalled_at` + log；不触发 round | `event_policies.message_recalled = "sync" \| "log_only"` |
| Membership | log + 触发"入群欢迎"模板（如果 channel 配了）；BotLeft 走清理 | `event_policies.membership = "auto_welcome" \| "log_only"` |
| ReadReceipt | log + 不动作 | `event_policies.read_receipt = "log" \| "drop"` |

策略字段加到 `ChannelAccountConfig` 上（[`channel/config.rs`](../../crates/ha-core/src/channel/config.rs)），默认值都走"log"——基础设施 PR 不需要这些字段，第二阶段 PR 才接。

## 4. PR 切分

### Phase B.0 — 基础设施 PR（仅 `Message` variant 实接，其它 log-only）

**包含**：

1. `InboundEvent` enum + 所有子结构定义（`channel/types.rs`）
2. `ChannelPlugin::start_account` 签名改 `mpsc::Sender<InboundEvent>`
3. 12 个 channel 插件全 `inbound_tx.send(InboundEvent::Message(msg))` 替换
4. `dispatcher` 改 `mpsc::Receiver<InboundEvent>` + match enum，非 Message 分支全 log-only
5. 现有 dispatcher 测试 mock 全部跟着改（约 6-8 处）
6. 飞书 ws_event.rs 测试加 `parse_message_event` 的回归覆盖（保护现有逻辑）

**不包含**：任何新事件解析、任何事件级业务行为、配置策略。

**测试**：现有飞书 / Telegram / Discord 入站消息端到端测试不退步（在 [`channel/worker/tests.rs`](../../crates/ha-core/src/channel/worker/tests.rs) 跑）。

**预估工作量**：1 天（机械替换为主）。

### Phase B.1 — 飞书新事件解析

**包含**：

1. 飞书 ws_event.rs 加 9 种新 event_type 解析 + 测试 fixture（每种事件 1 条真实/模拟 JSON）
2. `process_reaction` / `process_message_recalled` / `process_membership` / `process_read_receipt` 在 dispatcher 加 log_info + 埋点（接 LearningTracker）

**不包含**：业务行为（如 auto_welcome、session messages 表 sync）。

**预估工作量**：1.5 天。

### Phase B.2 — 业务行为接入（按事件类型分多个小 PR）

每个事件类型一个独立小 PR，例：

- `feat(channel/dispatch): MessageEdited 同步 session messages 表`
- `feat(channel/dispatch): MessageRecalled 标记 messages.recalled_at`
- `feat(channel/dispatch): Membership::BotLeft 清理 session + 解绑 project`
- `feat(channel/dispatch): Membership::UserJoined 触发 channel-级欢迎模板`
- `feat(channel/dispatch): Reaction 接入 ask_user 多选/确认快捷按钮（独立于 card action 旁路）`

按需推进，不阻塞 B.0/B.1 合并。

### Phase B.3 — 其它 channel 的事件接入

Telegram、Discord、Slack 等的 reaction/edit/recall 事件解析。同样每个 channel × 事件类型一个 PR，机械工作。

## 5. 测试策略

### 5.1 类型层

`channel/types.rs` 加 InboundEvent serde 单测：每个 variant round-trip 序列化 + 反序列化（保 frontend 接到的 JSON 形态稳定）。

### 5.2 dispatcher 层

[`channel/worker/tests.rs`](../../crates/ha-core/src/channel/worker/tests.rs) 加：

- `process_message_still_works` — 现有消息流不退步
- `non_message_events_are_logged_not_dropped` — 给 mock dispatcher 发各种 variant，看是否都进 process_xxx 分支（不需要后续动作，看到日志/计数即可）
- `reaction_does_not_start_agent_round` — 防退步，确认非消息事件**不**触发 chat engine

### 5.3 channel 层

每个 channel 的现有 inbound 测试改 mpsc::Receiver<InboundEvent> 后还要能跑通——重点是飞书 ws_event 的 41 个测试不退步。

### 5.4 端到端

桌面 dev 跑：飞书发条消息→看 dispatcher 日志收到 `InboundEvent::Message(...)`；之后 B.1 合后看到 `Reaction` log。

## 6. 风险与边角

### 6.1 跨 channel 的 worktree 撞车

记忆里：`.claude/worktrees/` 里若有别的 worktree 在改 channel 别 stash。本计划改 12 个 channel，开工前必须 `ls .claude/worktrees/` 确认没有人在做并行 channel 工作；有的话先和并行 worktree 协调。

### 6.2 dispatcher 反压

非消息事件（特别是 ReadReceipt）量大时可能淹没 dispatcher mpsc buffer（当前 [`channel/worker/dispatcher.rs:18`](../../crates/ha-core/src/channel/worker/dispatcher.rs#L18) `MAX_CONCURRENT_INBOUND = 20` 是消息处理的并发上限，不是 channel buffer 容量）。**对策**：在 `spawn_inbound_dispatcher` 创建 channel 时把 buffer 做大（256 → 1024）；非消息事件全部 log-only 处理 < 1ms，不会反压。

### 6.3 frontend 兼容

前端通过 `channel:message_update` event bus 拿通知，目前只接消息流。本计划 dispatcher 内非消息事件**不 emit** `message_update`——保持前端零改动。后续 B.2 真要 sync session messages 表时，会按需 emit。

### 6.4 序列化漂移

`InboundEvent` 用 `#[serde(tag = "kind")]`，前端如果以后要订阅 channel 入站事件（debug 面板？），JSON 里有稳定 `kind` 字段。所有 variant 都用 camelCase 字段名（与 MsgContext 一致）。**禁止**改成 snake_case 或 PascalCase——HTTP/WS payload 兼容性靠这个一致性吃饭。

### 6.5 持久化

InboundEvent 当前**不落库**（除了 message 已经走 channel/db）。如果后续要做"channel 入站事件历史"审计，需要新表 `channel_events` — 不在本计划范围。

## 7. 验收清单

Phase B.0 PR 合并前必须：

- [ ] 12 个 channel 插件 `cargo check` 全过，`cargo test -p ha-core` 全过
- [ ] 现有飞书 41 个测试 + 现有 dispatcher 测试零退步
- [ ] grep 验证 `mpsc::Sender<MsgContext>` 在 channel 子树内**完全消失**（避免漏改）
- [ ] grep 验证 `inbound_tx.send(MsgContext` 全替换为 `inbound_tx.send(InboundEvent::Message(`
- [ ] 桌面 dev 实地：飞书 / Telegram / Discord 各发一条消息能正常 dispatch
- [ ] [api-reference.md](../architecture/api-reference.md) 不需要更新（trait 改动是 internal，不暴露到 Tauri/HTTP）
- [ ] 文档：[architecture/channel-system.md](../architecture/) 如果有这份文档（当前未确认存在），更新；没有则不强加

## 8. 回滚预案

如果 B.0 合并后发现某个 channel 实地不工作：

1. 先确认是 enum 改动还是该 channel 自身的事件源 bug
2. enum 改动：`git revert <merge>` 撤回基础设施 PR；事件源 bug：单独 channel 修
3. 不要在 enum 上做 backward-compat 包装（按记忆里"破坏性改动直接 drop，不要保留迁移/兼容路径"）

---

## 9. 与下游的协同

Phase B 合入后是 [Phase C 飞书业务工具](feishu-business-tools.md)的前置依赖之一吗？**不是**——Phase C 走 tools 通道而不是 channel inbound 通道。两者独立，可以并行。但如果 Phase C 的某个 tool 想响应"用户在飞书群里点了 reaction"，那确实要 B 先就位——按需协同。
