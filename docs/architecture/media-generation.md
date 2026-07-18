# 媒体生成子系统（Media Generation）

> 返回 [文档索引](../README.md)
>
> 统一的图片 / 音频生成服务商体系（`video` 模态预留）：**服务商 → 多模型 → 功能默认链**三层，
> 镜像 [STT 子系统](stt.md)的「独立 provider 列表」模式。取代旧的 `image_generate` /
> `audio_generate` 固定槽位配置（每 vendor 一个写死槽、每槽单模型、能力写死在 Rust trait），
> 旧结构已直接 drop、无迁移。
>
> 代码入口：[`crates/ha-core/src/media_gen/`](../../crates/ha-core/src/media_gen/)。

## 1. 定位与三层模型

```
MediaGenConfig (AppConfig.media_gen)
├── providers: Vec<MediaProviderConfig>     ← 用户自管服务商列表（顺序 = auto 候选优先级）
│   └── models: Vec<MediaModelConfig>       ← 一个服务商挂多模型（凭据配一次）
│       ├── modality: image | audio | video(预留)
│       ├── image: Option<ImageModelCaps>   ← 数据驱动能力（非 trait 写死）
│       └── audio: Option<AudioModelCaps>
├── chains: MediaDefaultChains              ← 功能默认链 ×4：image / speech / music / sfx
│   └── Option<MediaModelChain { primary, fallbacks }>   （None = auto）
├── image_defaults: ImageGenDefaults        ← enabled / timeout(180s) / default_size / AR / resolution
└── audio_defaults: AudioGenDefaults        ← enabled / timeout(300s) / default_duration
```

- **服务商**（`MediaProviderConfig`）：`id`（UUID）/ `name` / `kind: MediaVendorKind`（决定
  adapter 路由）/ `base_url` / `api_key` / `enabled` / `models` / `default_voice` /
  `allow_private_network`（SSRF 放行内网，自建端点用）/ `extra`（当 secret 处理）。
  `masked()` 遮蔽 api_key + extra；`is_usable()` = enabled + 有凭据（OpenAI-compatible
  自建端点允许无 key、有 base_url 即可）。
- **模型**（`MediaModelConfig`）：扁平 struct + 可选能力组（非 tagged enum，serde 对前端
  友好）。`ImageModelCaps`：max_n / supports_{size,aspect_ratio,resolution} + 合法值枚举
  （空 = 不限）/ `supports_mask`（inpaint 蒙版，仅 OpenAI gpt-image 系）/ `edit`（img2img
  能力组）。`AudioModelCaps`：kinds（speech/music/sfx）/ supports_duration / needs_voice /
  default_voice / min·max_duration。**caps = None 的自填模型宽松放行**（不被能力闸门误杀，
  provider 端自然报错计入 failover log）。
- **功能默认链**：audio 按 kind 三条独立链（TTS 模型做不了 music，单链 + 过滤语义模糊）；
  `MediaModelRef` Display 为 `"provider_id::model_id"`（对齐 `ActiveModel` 约定）。

### vendor 目录（`MediaVendorKind`）

| kind | adapter | 模态 | 备注 |
| --- | --- | --- | --- |
| `openai` | image + audio | 图 + TTS | `/v1/images/generations\|edits` + `/v1/audio/speech`；gpt-image 系支持 mask inpaint |
| `google` | image | 图 | Gemini / Imagen；`thinking_level` 走模型级 `extra` |
| `fal` / `minimax` / `siliconflow` / `zhipu` / `tongyi` | image | 图 | 能力矩阵各异（见 catalog） |
| `elevenlabs` | audio | TTS + Music + SFX | voices 实时拉取 |
| `openai-compatible` | image + audio | 自建 | 必填 base_url，复用 openai adapter，可无 key |

## 2. 模块布局

```
media_gen/
├── mod.rs        门面 re-exports
├── types.rs      §1 全部数据结构 + serde + masked + is_usable/serves
├── catalog.rs    内置模板 + 预设模型目录（单一真相源，取代旧 trait capabilities()/
│                 audio_model_catalog()/前端硬编码预设；GUI-only，经命令下发不进 config）
├── crud.rs       写助手（镜像 stt/crud.rs）：add/update(masked-key 保护)/delete(清悬挂链)/
│                 reorder/set_media_default_chain(校验 modality+kind)/update_defaults；
│                 mutate_config 标签 media_gen.*
├── resolve.rs    候选解析单一入口 resolve_candidates（显式 model pid::mid 消歧 → 链 → auto）
│                 + validate_image_request（数据驱动能力校验）
├── execute.rs    统一 failover 执行器 execute_image/execute_audio（全部消费方共用）
├── input.rs      参考图加载（路径/URL/data-uri，SSRF-gated，≤5 张坏项跳过）
├── overview.rs   sanitized 可用性/能力视图（无凭据，供 design 对话框 + 工具设置提示）
├── probe.rs      测试连接探针（轻 GET，per-vendor 端点，audio 探针 = voices/models）
├── voices.rs     voice 目录（elevenlabs 实时 + 10min 凭据指纹缓存按 provider_id 隔离；
│                 openai 系静态表 OPENAI_TTS_VOICES）
└── adapters/     wire 协议实现（trait 只剩 generate；身份/默认模型/能力全数据化）
    ├── image/{openai,google,fal,minimax,siliconflow,zhipu,tongyi}.rs
    └── audio/{openai,elevenlabs}.rs
```

## 3. 运行时解析与执行（红线）

**唯一候选解析入口 `resolve.rs::resolve_candidates(cfg, function, explicit_model)`**，优先级：

1. **显式 model**（工具 `model` 参数）：`"provider::model"` 精确 pin；裸 model id 须全局唯一，
   撞名报错要求 `pid::mid` 形式。**pin = 不 failover**（沿袭旧工具行为）。
2. **已配置链**：primary → fallbacks，悬挂 / 不可用引用 `app_warn` 跳过；**链耗尽即失败、
   绝不滑落 auto**（可预测性——用户 pin 了链就不该悄悄用别的服务商）。
3. **auto**：providers 顺序 × 能力过滤，每 provider 取首个匹配模型（避免同 provider 排队
   多模型拖长失败链）。

**唯一执行入口 `execute.rs::execute_image / execute_audio`**：逐候选 → 宽松能力校验
（`validate_image_request`，mask 请求只投 `supports_mask` 模型）→ 每候选至多 1 次可重试
错误重试（`failover::classify_error` + `retry_delay_ms(attempt, 2000, 10000)`）→ 下一候选；
**每次 attempt 记账**（`KIND_IMAGE_GENERATION` / `KIND_AUDIO_GENERATION`，metadata 含
size/n/AR/res/kind/duration/attempt；provider_id 为 UUID、provider_name 为用户显示名）。
SSRF：执行器对每候选 base URL 过 `check_url`（策略 = `allow_private_network` ? AllowPrivate
: 全局默认）；audio adapter 对最终 URL 再检一次（同策略）。

**三处消费全部走执行器，禁止再各写 provider 循环**（旧版聊天工具 / design image / design
audio 三份重复 failover 是本次重构消灭的对象）：

| 消费方 | 入口 | operation |
| --- | --- | --- |
| 聊天 `image_generate` 工具 | `tools/image_generate/generate.rs` | `tool.image_generate` |
| 聊天 `audio_generate` 工具 | `tools/audio_generate/mod.rs` | `tool.audio_generate` |
| design `image` 产物 + inpaint | `design/image.rs::generate_image_parts` | `design.image` |
| design `audio` 产物 | `design/audio.rs::generate_audio_parts` | `design.audio` |

**voice 三层覆盖**：调用级（工具 `voice` 参数 / design `audioVoice`）> 模型级
`audio.default_voice` > provider 级 `default_voice` > adapter 内置兜底（alloy / Rachel）。
不设全局 voice——voice id 是 provider 语境的，跨 provider 全局默认无意义。

**超时**：`image_defaults.timeout_seconds`（默认 180）/ `audio_defaults.timeout_seconds`
（默认 300），读侧 `effective_timeout_secs()` clamp `[30, 900]`——不回写持久层。

## 4. Agent 工具面

- **`image_generate`**（`async_capable`）：args `action(generate|list)/prompt/image(s)/size/
  aspectRatio/resolution/n/model`。schema 动态（`get_image_generate_tool_dynamic(&MediaGenConfig)`）：
  描述列链感知候选 + 数据 caps 汇总；注入门控 `image_defaults.enabled && has_capable_provider(Image)`
  （无 provider 不注入）。
- **`audio_generate`**（`async_capable`，本次新增为聊天工具）：args `action/prompt/
  kind(speech|music|sfx 默认 speech)/voice/durationSeconds/model`；显式 kind > `[music]`/`[sfx]`
  prompt 前缀 > speech；产物落 attachments，`__MEDIA_ITEMS__` 携 `MediaItem{kind:"file",
  mimeType:"audio/*"}` 复用现有 FileCard → FilePreviewPane `<audio controls>` 播放通路
  （**刻意不加 MediaKind::Audio 变体**）。**红线：计费副作用，绝不进
  `async_jobs::retry::is_retry_eligible`**。ha-server 媒体透传白名单（`routes/sessions.rs`）
  已含 `audio_generate`。
- 两工具均入 `is_design_scope_tool` 白名单（design 对话可生成素材）。
- **design 工具 / `CreateArtifactInput`** 参数透传：`image_size` / `image_resolution` /
  `aspect_ratio`（image）、`audio_kind` / `audio_voice` / `audio_duration_secs`（audio）。

## 5. Owner 命令面（Tauri ↔ HTTP，详表见 [api-reference.md](api-reference.md)）

Provider CRUD 即时保存；工具面板只写链 + defaults——**分段接口，防两面板整文档快照互踩**。

| 命令 | HTTP | 说明 |
| --- | --- | --- |
| `get_media_gen_config` | `GET /api/config/media-gen` | Tauri 未脱敏（本机信任域，对齐 `get_stt_providers`）；**HTTP masked** |
| `add/update/delete_media_provider` | `POST/PUT/DELETE /api/config/media-gen/providers[/{id}]` | update 走 masked-key 保护（掩码值不覆写真值） |
| `reorder_media_providers` | `PUT .../providers/reorder` | 顺序 = auto 优先级 |
| `set_media_default_chain` | `PUT .../chains/{function}` | function ∈ image/speech/music/sfx；chain=null 清回 auto |
| `update_media_gen_defaults` | `PUT .../defaults` | 两 defaults 整体保存 |
| `get_media_provider_templates` | `GET .../templates` | catalog（GUI-only） |
| `list_media_voices` | `GET .../voices?providerId=` | 按 vendor 能力分派 |
| `test_media_provider` | `POST .../test` | 保存前草稿（kind+key+baseUrl）或已存 provider（id） |
| `get_media_gen_overview` | `GET .../overview` | sanitized，无凭据 |

## 6. 设置三件套 + 前端

- **GUI**：模型服务商设置页（`ModelConfigPanel`）第 5 Tab「生成模型」`mediaModels`
  （`src/components/settings/media-gen/`：服务商卡列表 dnd 排序 + 模板添加对话框 + 模型/
  能力编辑 + 测试连接 + voices 拉取）；工具设置页「媒体生成」Tab `mediaGenerate`
  （`MediaGeneratePanel`：启用开关 + 四条链 ModelChainEditor + 默认参数 + 超时；合并取代旧
  imageGenerate / audioGenerate 两 Tab——单一 `MediaGenConfig` 文档被两面板各持快照保存会互踩）。
- **`ha-settings`**：category `media_generation`（LOW）。读：providers 逐个 `masked()` +
  chains/defaults 原样。**写只放行 `chains` / `imageDefaults` / `audioDefaults` 三段**，
  payload 含 `providers` 一律报错指向 owner UI——凭据可写 = 模型可植入自己的 key / 外泄端点。
  链写入经 `check_serves_function` 校验。
- **reset**：`settings_reset` section `media_gen`（scope tools）——只重置 chains + defaults，
  **providers（凭据）保留**（对齐旧 reset 保 api_key 的契约）。
- 深链：`openMediaModelSettings()`（`settings:navigate` + `modelTab: "mediaModels"`；App 层
  监听须透传 `modelTab`）。设计空间图/音生成对话框在无可用 provider 时渲染空态 + 该深链。

## 7. 记账与可观测

- 用量：执行器内每 attempt 记 `KIND_IMAGE_GENERATION` / `KIND_AUDIO_GENERATION`（生成类无
  token，只记次数 + 耗时，禁字符估算；无痕会话经 session_id 归零入账遵全局契约）。
  **provider_id 自本次起为 UUID**（旧固定串 "openai" 等的历史行按无迁移政策断组）。
- 日志：稳定 category `media_gen`（source `resolve` / `execute` / `load_input_images`），
  工具层沿用 `tool` / `image_generate`、`tool` / `audio_generate`；design 层 `design` /
  `image`、`design` / `audio`。
- 存量下载目录 `~/.hope-agent/image_generate/` **不改名**（改了会断历史 mediaUrls）。

## 8. 关键设计决策

| # | 决策 | 理由 |
| --- | --- | --- |
| 1 | 统一媒体服务商列表而非图/音两套或并入 LLM ProviderConfig | 一个 OpenAI 条目同时挂生图 + TTS 模型（key 配一次）；`ApiType` 是聊天协议枚举，塞纯生成商语义混乱 |
| 2 | 能力全数据化（catalog 模板 → config），trait 只剩 `generate` | 用户可自填新模型不用等发版；能力矩阵可视化、可校验 |
| 3 | audio 默认链按 kind 三条 | 三 kind 模型集几乎不相交，单链 + 过滤 = 每次过滤剩一条，配置语义反而模糊 |
| 4 | 链耗尽不滑落 auto | 用户显式 pin 的链滑到未选服务商 = 不可预测扣费 |
| 5 | caps=None 宽松放行 | 自填模型不被闸门误杀；代价是坏参数烧一次真实调用，可接受 |
| 6 | `supports_mask` 独立于 `edit` | OpenAI 有 mask inpaint 无通用 img2img（旧 trait 声明 edit 禁用但 inpaint 只有它支持的矛盾，数据化后显式表达）；其它 vendor 收到 mask 会静默整图重生成，必须过滤 |
| 7 | video 只留 modality 枚举 | 结构不 churn；不做 adapter / 模板 / UI |
| 8 | 旧配置直接 drop | 项目既定无迁移政策；升级后 providers 为空 → 工具门控自动收回 + 各处空态引导重配 |
