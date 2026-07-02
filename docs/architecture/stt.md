# 语音转写（STT）

> 返回 [文档索引](../README.md) | 关联源码：[`crates/ha-core/src/stt/`](../../crates/ha-core/src/stt/)、[`src-tauri/src/commands/stt.rs`](../../src-tauri/src/commands/stt.rs)、[`crates/ha-server/src/routes/stt.rs`](../../crates/ha-server/src/routes/stt.rs)、IM 集成在 [`channel/types.rs`](../../crates/ha-core/src/channel/types.rs)

## 概述

STT（Speech-to-Text）是一个**独立配置、独立鉴权、独立错误分类**的语音转写引擎，与主 LLM Provider 列表**物理隔离**——理由与 Memory 的 Embedding 子系统一致：转写模型有自己的语义维度（按分钟计费、是否支持流式、语种覆盖），且多协议表面（OpenAI multipart、chat-completions ASR、5 种 WebSocket 方言）无法塞进 LLM 的 `provider::ApiType` 枚举。

提供三类能力：

- **桌面语音输入**：一次性 batch 转写（`stt_transcribe_blob`）+ 流式会话（边说边出字，`SttSessionManager`）
- **IM 自动转写**：账号级 opt-in，把入站语音消息转成文本注入对话（**batch-only**）
- **本地后端**：whisper.cpp / faster-whisper / FunASR / sherpa-onnx 一键接入（OpenAI 兼容端点 + `allow_private_network`）

所有配置写入经唯一入口 `stt::crud` 走 `mutate_config` 并 emit `config:changed`，与 LLM Provider 写入契约同构。

## 模块结构

核心全在 `crates/ha-core/src/stt/`（零 Tauri 依赖）：

| 文件 | 职责 |
|---|---|
| `mod.rs` | 子系统根、公共 API 再导出、`voice_prefix_for_locale()` 本地化前缀助手 |
| `types.rs` | 数据模型：`SttConfig` / `SttProviderConfig` / `SttModelConfig` / `SttProviderKind` / `ActiveSttModel` / `Transcript*`；硬常量 `MAX_BATCH_AUDIO_BYTES` |
| `engine.rs` | 按 `SttProviderKind` 分发 + Failover 编排：`resolve_active` / `current_desktop_chain` / `current_im_chain` / `failover_transcribe_batch` |
| `errors.rs` | `SttError` 分类错误 + `is_retriable()`（gates failover） |
| `local.rs` | 已知本地后端目录（`KnownLocalSttBackend` + 4 个后端 key 常量 + `probe_local_backend_alive`） |
| `session.rs` | 流式会话管理器 `SttSessionManager`（WebSocket 生命周期 + EventBus 集成 + idle GC） |
| `crud.rs` | 唯一写入口（`add/update/delete/reorder` provider、`set_active`/fallback 链、`upsert_known_local_stt_provider`） |
| `providers/` | 各 wire 协议实现 + 共享 helper（`load_batch_audio` / `ws_to_https_twin` / `ws_connect_with_caps` / 错误分类） |

## 配置模型

`AppConfig.stt: SttConfig`（[`config/mod.rs`](../../crates/ha-core/src/config/mod.rs)）：

```rust
pub struct SttConfig {
    pub providers: Vec<SttProviderConfig>,  // 云 + 本地合并列表
    pub active_model: Option<ActiveSttModel>,// 桌面语音输入主模型（failover 链首）
    pub fallback_models: Vec<ActiveSttModel>,// 桌面 failover 链（active 失败时按序尝试）
    pub im_fallback_model: Option<ActiveSttModel>, // IM 自动转写专用，未设回退到 active_model
    pub default_options: TranscriptOptions,  // 默认转写选项
}
```

- **`SttProviderConfig`**：`id` / `name` / `kind: SttProviderKind` / `base_url` / `api_key`（legacy 单 key）/ `auth_profiles: Vec<AuthProfile>`（与 LLM Provider 共用 key 轮换）/ `models: Vec<SttModelConfig>` / `enabled` / `allow_private_network` / `extra: HashMap`（provider 私有 secret，如 app_id / cluster / region）
- **`SttModelConfig`**：`id` / `name` / `supports_streaming` / `languages` / `cost_per_minute` / `supports_timestamps` / `supports_diarization`
- **凭据脱敏**：`SttProviderConfig::masked()` 对 `api_key` / `auth_profiles` / `extra` 三处 secret 统一打码（`xxx...yyy` / 短值 `****`）；写回时经 `is_masked_key` + `merge_profile_keys`（复用 `provider::`）**合并保密**——前端 round-trip 把打码值传回不会清空真实 key

## Provider 抽象（`SttProviderKind`）

8 个 wire 协议变体，`engine` 按 kind 分发到对应 `providers/*` 实现：

| 变体 | 协议 | 流式 | Batch | 典型 |
|---|---|---|---|---|
| `OpenaiTranscriptions` | HTTP multipart `/v1/audio/transcriptions` | ✗ | ✓ | OpenAI Whisper |
| `OpenaiCompatible` | HTTP multipart | 视上游 | ✓ | Groq / 四个本地后端 / StepFun / SiliconFlow |
| `OpenaiChatCompletionsAsr` | HTTP JSON（chat/completions + input_audio） | ✗ | ✓ | DashScope Qwen3-ASR / gpt-4o-audio |
| `DeepgramWs` | WebSocket 二进制 | ✓ | ✗ | Deepgram realtime |
| `AssemblyaiWs` | WebSocket 二进制 | ✓ | ✗ | AssemblyAI realtime |
| `AzureWs` | WebSocket（USP 协议） | ✓ | ✗ | Azure Speech |
| `VolcengineWs` | WebSocket 二进制 | ✓ | ✗ | 火山 / 字节 ASR |
| `XunfeiWs` | WebSocket（hmac-sha256 签名 URL） | ✓ | ✗ | 讯飞 IAT |

helper 方法：`default_base_url()` / `supports_streaming()` / `supports_batch()` / `uses_multipart_upload()` / `display_name()`。`supports_batch()` 是 **fallback / IM 链白名单**的判定依据——WS-only kind 不能进 batch 链（见安全红线）。

## Failover

`failover_transcribe_batch(primary, fallback_chain, audio, options)` 按序尝试主模型 → fallback 链；`SttError::is_retriable()` 裁决是否换下一个：

- `UnsupportedAudio` **短路**（音频格式问题换模型也没用，立即终止）
- `Network` / `RateLimit` / `Auth` / `ProviderUnavailable` / `Other` → 尝试链中下一个
- 全链耗尽返回 `FailoverError`（含所有 `AttemptedModel` 记录 + 终态错误，供遥测）

`SttError` 10 变体：`NotFound` / `NoActiveModel` / `Auth` / `RateLimit` / `Network` / `UnsupportedAudio` / `ProviderUnavailable` / `SsrfBlocked` / `Io` / `Other`。`Display` 渲染为 `stt:<code>: <message>`，便于 Tauri / HTTP 边界两侧解析 code。桌面链 = `current_desktop_chain()`（active + fallback_models），IM 链 = `current_im_chain()`（im_fallback_model 或 active，仅 batch-capable）。

知识空间资料舱的 `audio_transcript` / `video_transcript` 导入复用桌面链：owner-plane import 接收用户选择的音视频字节，调用 `failover_transcribe_batch(current_desktop_chain, AudioPayload::Bytes, default_options)`，默认请求 timestamps（若用户未显式配置），成功后只保存带 provenance / provider / model / language / duration / segment 时间戳的 Markdown 转录快照；原始媒体不持久化。未配置 STT 或转录失败时，对应 import item 进入 `failed`，错误保留在导入历史供用户重试。

## 本地后端

`stt::local` 维护 4 个已知本地后端目录（`KnownLocalSttBackend`），全部用 `OpenaiCompatible` kind 接 OpenAI 兼容端点：

| 后端 | key | 默认端点 | 端口 |
|---|---|---|---|
| whisper.cpp | `whisper-cpp` | `http://127.0.0.1:8080` | 8080 |
| faster-whisper-server | `faster-whisper` | `http://127.0.0.1:8000` | 8000 |
| FunASR | `funasr` | `http://127.0.0.1:10097` | 10097 |
| sherpa-onnx | `sherpa-onnx` | `http://127.0.0.1:6006` | 6006 |

- **探测**：`probe_local_backend_alive()` 对 `127.0.0.1:{port}` 做 500ms TCP connect
- **一键接入**：`upsert_known_local_stt_provider()` 幂等——按 host/port 匹配既有 provider（`known_local_stt_backend_matches`），命中则补模型 + 启用，未命中则新建；强制 `allow_private_network = true` 让其能打 localhost
- 前端"是否已配本地后端"必须消费此 catalog，**禁止硬编码 regex**（与 LLM `provider::local` 同纪律）

## 流式会话

`SttSessionManager`（全局单例）管理流式转写生命周期，**纯内存、无 DB 持久化**：

| 方法 | 行为 |
|---|---|
| `start(provider?, model?, options)` | 经 `resolve_active` 解析 (provider, model, profile)，开上游流，spawn 事件泵，返回 `stt_{uuid}` |
| `push_chunk(session_id, bytes)` | 经 `try_send` 推音频到上游（不跨锁 clone）；每 32 chunk coalesce 一次 `last_active`；buffer 满返 `Network`，会话已逐出返 `NotFound` |
| `finalize(session_id)` | 锁内 drop audio_tx 发 EOS，移除会话，**30s 超时**等最终 transcript |
| `cancel(session_id)` | 置 cancel flag + drop audio_tx + 移除 |
| `gc_idle()` | 逐出空闲 > `SESSION_IDLE_TIMEOUT_SECS`（**300s**）的会话；由 `runtime_tasks` 周期调用；`app_warn!("stt", ...)` 留痕 |

事件泵（`spawn_event_pump`）把上游 delta 累积成最终 transcript 并 emit 到 EventBus（EventBus 不可用时静默丢弃、不崩溃）：

```rust
pub const EVENT_TRANSCRIPT_PARTIAL: &str = "stt:transcript_partial";
pub const EVENT_TRANSCRIPT_FINAL:   &str = "stt:transcript_final";
pub const EVENT_SESSION_ERROR:      &str = "stt:session_error";
```

## IM 自动转写

账号级 opt-in，把入站语音消息转成文本：

- **开关**：`ChannelAccountConfig::auto_transcribe_voice()` 读 `settings.autoTranscribeVoice`（key 常量 `SETTINGS_KEY_AUTO_TRANSCRIBE_VOICE`，**默认 `false`**、per-account）
- **本地化前缀**：`voice_prefix_for_locale(locale, text)` 给转写文本加 `[语音转写] …`（覆盖 12 种本地化标签，未知 locale 回退英文）
- **链路**：开启后语音消息走 `im_fallback_model` → `active_model` → `fallback_models`，统一经 `failover_transcribe_batch`——**batch-only，IM 不用流式**

## 命令 / 路由面

| 平面 | 入口 | 数量 | 脱敏 |
|---|---|---|---|
| Tauri 命令 | [`src-tauri/src/commands/stt.rs`](../../src-tauri/src/commands/stt.rs) | 20 | **unmasked**（桌面 = 本机信任域） |
| HTTP 路由 | `/api/stt/*`（[`routes/stt.rs`](../../crates/ha-server/src/routes/stt.rs)） | 17 | **masked**（响应内 provider secret 打码） |

分组（两端镜像）：provider CRUD、active / fallback / im-fallback 选择、本地 catalog（list / probe / upsert）、转写（`transcribe_blob` 一次性 + session `start`/`push_chunk`/`finalize`/`cancel`）。

## 安全红线

- **Size caps（fail-closed）**：`MAX_BATCH_AUDIO_BYTES = 25 MiB`（对齐 OpenAI Whisper 上限），在 **Tauri 命令 / HTTP 路由 / provider `load_batch_audio` 三处**校验（base64 长度 + 解码后字节都查，超限前不分配大 buffer）；流式 chunk 经 `MAX_PUSH_CHUNK_BYTES = 1 MiB`（命令层）预检 base64 长度；WS 帧 `WS_MAX_MESSAGE_BYTES = 4 MiB`、流通道 `STT_STREAM_CHANNEL_CAPACITY = 64`
- **SSRF**：所有 provider URL 经 `security::ssrf::check_url`；本地后端 `allow_private_network = true` 才放行 localhost；**WS provider 先经 `ws_to_https_twin` 转 http(s) 孪生 URL 再过 SSRF**（`check_url` 不认 ws/wss scheme）；批量 provider 显式 `redirect::Policy::none()` 防 3xx 跳到内网 / metadata
- **batch-capable guard**：`check_batch_capable()` 拦截把 `fallback_models` / `im_fallback_model` 设成 WS-only provider（桌面 `active_model` 可用 WS 走流式，IM / fallback 链 batch-only 必拒）
- **fail-closed 选择**：无 active model → `SttError::NoActiveModel`，不回退"任意模型"，必须显式配置
- **会话清理**：idle 300s 逐出，防废弃 WebSocket 泄漏 provider 带宽
- **incognito**：STT 配置是全局（非会话级），当前**未与无痕模式集成**——无 ephemeral 配置概念

## 设置（Settings）约定

STT 归「**强制留 GUI 的例外**」同类（凭据安全）：

- **Provider 列表 + Key owner-GUI-only**：provider 写入只经 Tauri / HTTP owner 命令（调 `stt::crud`），**不进 `update_settings`**
- **`get_settings` 只读摘要**：暴露脱敏 `stt` 块（`providerCount` / `activeModel` / `fallbackCount` / `imFallbackConfigured` / `imAutoTranscribeAccountCount`），不泄 key
- **唯一模型可写旋钮**：`im_auto_transcribe`（per-account 语音转写开关，**LOW**，`update_settings` 经 `update_im_auto_transcribe`）

## 跨子系统

| 子系统 | 关系 |
|---|---|
| Config | `AppConfig.stt`；写经 `mutate_config` + `config:changed` |
| Channel | per-account `auto_transcribe_voice`；IM worker 命中语音消息时调 STT + `voice_prefix_for_locale` |
| Provider | 复用 `AuthProfile` key 轮换 + `apply_proxy` + `is_masked_key` / `merge_profile_keys` |
| Security / SSRF | 每个出站 URL 过 `check_url`；WS 经孪生 URL |
| EventBus | 流式会话经 `get_event_bus()` emit `stt:*` |
| Runtime tasks | 周期调 `SttSessionManager::gc_idle()` 逐出空闲会话 |

## 关键文件索引

| 文件 | 角色 |
|---|---|
| [`crates/ha-core/src/stt/mod.rs`](../../crates/ha-core/src/stt/mod.rs) | 子系统根 + 公共 API + `voice_prefix_for_locale` |
| [`crates/ha-core/src/stt/types.rs`](../../crates/ha-core/src/stt/types.rs) | 数据模型 + `SttProviderKind` + `masked()` + size 常量 |
| [`crates/ha-core/src/stt/engine.rs`](../../crates/ha-core/src/stt/engine.rs) | 按 kind 分发 + `failover_transcribe_batch` + 桌面 / IM 链 |
| [`crates/ha-core/src/stt/errors.rs`](../../crates/ha-core/src/stt/errors.rs) | `SttError` + `is_retriable` |
| [`crates/ha-core/src/stt/local.rs`](../../crates/ha-core/src/stt/local.rs) | 4 本地后端 catalog + probe + upsert |
| [`crates/ha-core/src/stt/session.rs`](../../crates/ha-core/src/stt/session.rs) | `SttSessionManager` + `stt:*` 事件 + idle GC |
| [`crates/ha-core/src/stt/crud.rs`](../../crates/ha-core/src/stt/crud.rs) | 唯一写入口 + `check_batch_capable` |
| [`crates/ha-core/src/stt/providers/`](../../crates/ha-core/src/stt/providers/) | 8 协议实现 + 共享 batch / WS helper |
| [`src-tauri/src/commands/stt.rs`](../../src-tauri/src/commands/stt.rs) | 20 Tauri 命令（unmasked） |
| [`crates/ha-server/src/routes/stt.rs`](../../crates/ha-server/src/routes/stt.rs) | 17 HTTP 路由（masked） |
