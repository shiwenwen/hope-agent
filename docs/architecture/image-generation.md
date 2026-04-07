# 图片生成工具技术架构文档

> 返回 [文档索引](../README.md)

## 概述

OpenComputer 的图片生成系统采用 **Trait 抽象 + Capabilities 声明 + 动态工具描述 + 自动降级** 的架构，支持 7 个内置 Provider，覆盖文生图和参考图编辑两种模式。整个系统遵循「上层不感知 Provider」原则——工具入口函数 `tool_image_generate()` 通过统一的 `ImageGenProviderImpl` trait 与所有 Provider 交互，不包含任何 Provider 特定逻辑。

```mermaid
graph TD
    LLM["LLM Tool Call<br/>image_generate(prompt, image, ...)"]
    ENTRY["tool_image_generate()<br/>参数解析 → 图片加载 → 能力校验<br/>→ failover 循环 → 保存输出"]
    TRAIT["Box&lt;dyn ImageGenProviderImpl&gt;<br/>.capabilities() → 能力声明<br/>.generate(params) → 统一结果"]

    LLM --> ENTRY
    ENTRY -- "resolve_provider(id)" --> TRAIT

    TRAIT --> P1["OpenAI"]
    TRAIT --> P2["Google"]
    TRAIT --> P3["Fal"]
    TRAIT --> P4["MiniMax"]
    TRAIT --> P5["SiliconFlow"]
    TRAIT --> P6["ZhipuAI"]
    TRAIT --> P7["Tongyi"]

    style LLM fill:#e0e7ff,stroke:#4f46e5
    style ENTRY fill:#fef3c7,stroke:#d97706
    style TRAIT fill:#d1fae5,stroke:#059669
```

---

## 核心类型系统

### Provider Trait

```rust
pub(crate) trait ImageGenProviderImpl: Send + Sync {
    fn id(&self) -> &str;                          // "openai", "google", ...
    fn display_name(&self) -> &str;                // "OpenAI", "Google", ...
    fn default_model(&self) -> &str;               // "gpt-image-1", ...
    fn capabilities(&self) -> ImageGenCapabilities; // 能力声明
    fn generate<'a>(
        &'a self, params: ImageGenParams<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<ImageGenResult>> + Send + 'a>>;
}
```

每个 Provider 是一个零大小 unit struct（如 `pub(crate) struct OpenAIProvider;`），实现此 trait 的 5 个方法。

### Capabilities 声明

```rust
pub(crate) struct ImageGenCapabilities {
    pub generate: ImageGenModeCapabilities,   // 文生图能力
    pub edit: ImageGenEditCapabilities,       // 图片编辑能力
    pub geometry: Option<ImageGenGeometry>,   // 几何约束
}

pub(crate) struct ImageGenModeCapabilities {
    pub max_count: u32,              // 单次最多生成图片数
    pub supports_size: bool,         // 是否支持自定义尺寸
    pub supports_aspect_ratio: bool, // 是否支持 aspectRatio 参数
    pub supports_resolution: bool,   // 是否支持 resolution 参数
}

pub(crate) struct ImageGenEditCapabilities {
    pub enabled: bool,               // 是否支持编辑
    pub max_count: u32,              // 编辑模式最多输出图片数
    pub max_input_images: u32,       // 最多接受参考图数量
    pub supports_size: bool,
    pub supports_aspect_ratio: bool,
    pub supports_resolution: bool,
}

pub(crate) struct ImageGenGeometry {
    pub sizes: Vec<&'static str>,          // ["1024x1024", "1024x1536", ...]
    pub aspect_ratios: Vec<&'static str>,  // ["1:1", "16:9", ...]
    pub resolutions: Vec<&'static str>,    // ["1K", "2K", "4K"]
}
```

`validate_capabilities()` 在 failover 循环内自动校验：若某 Provider 不支持当前请求的参数组合（如 OpenAI 不支持编辑），则跳过该 Provider 并记录 failover 日志，无需上层感知。

### 统一参数与结果

```rust
pub(crate) struct ImageGenParams<'a> {
    pub api_key: &'a str,
    pub base_url: Option<&'a str>,
    pub model: &'a str,
    pub prompt: &'a str,
    pub size: &'a str,                    // "1024x1024"
    pub n: u32,                           // 生成数量
    pub timeout_secs: u64,
    pub extra: &'a ImageGenProviderEntry,  // Provider 特定配置（如 Google thinkingLevel）
    pub aspect_ratio: Option<&'a str>,    // "1:1", "16:9", ...
    pub resolution: Option<&'a str>,      // "1K", "2K", "4K"
    pub input_images: &'a [InputImage],   // 参考图（编辑模式）
}

pub(crate) struct InputImage {
    pub data: Vec<u8>,   // 原始字节
    pub mime: String,     // "image/png", "image/jpeg", ...
}

pub(crate) struct ImageGenResult {
    pub images: Vec<GeneratedImage>,
    pub text: Option<String>,  // 伴随文本（Gemini 会返回文字说明）
}

pub(crate) struct GeneratedImage {
    pub data: Vec<u8>,
    pub mime: String,
    pub revised_prompt: Option<String>,
}
```

---

## 7 个内置 Provider

### 能力矩阵

| Provider | ID | 默认模型 | 最大数量 | 编辑 | 参考图上限 | Size | AspectRatio | Resolution |
|----------|-----|---------|---------|------|-----------|------|-------------|------------|
| **OpenAI** | `openai` | `gpt-image-1` | 4 | - | - | 3 种 | - | - |
| **Google** | `google` | `gemini-3.1-flash-image-preview` | 4 | **5 张** | 5 | 5 种 | 10 种 | 1K/2K/4K |
| **Fal** | `fal` | `fal-ai/flux/dev` | 4 | **1 张** | 1 | 5 种 | 5 种 | 1K/2K/4K |
| **MiniMax** | `minimax` | `image-01` | 9 | **1 张** | 1 | - | 8 种 | - |
| **SiliconFlow** | `siliconflow` | `Qwen/Qwen-Image` | 4 | **1 张** | 1 | 8 种 | - | - |
| **ZhipuAI** | `zhipu` | `cogView-4-250304` | 1 | - | - | 6 种 | - | - |
| **Tongyi Wanxiang** | `tongyi` | `wanx-v1` | 4 | **1 张** | 1 | 3 种 | - | - |

### 支持尺寸详情

| Provider | 支持尺寸 |
|----------|---------|
| OpenAI | 1024x1024, 1024x1536, 1536x1024 |
| Google | 1024x1024, 1024x1536, 1536x1024, 1024x1792, 1792x1024 |
| Fal | 1024x1024, 1024x1536, 1536x1024, 1024x1792, 1792x1024 |
| MiniMax | _不支持自定义尺寸_ |
| SiliconFlow | 1024x1024, 1328x1328, 1664x928, 928x1664, 1472x1140, 1140x1472, 1584x1056, 1056x1584 |
| ZhipuAI | 1024x1024, 1024x1536, 1536x1024, 1024x1792, 1792x1024, 2048x2048 |
| Tongyi | 1024x1024, 720x1280, 1280x720 |

### 各 Provider 编辑实现方式

| Provider | 编辑机制 | 请求字段 | 模型切换 |
|----------|---------|---------|---------|
| **Google** | `inlineData` 多模态部件 | `contents.parts[].inlineData` | 不切换，同一模型 |
| **Fal** | data URI + 路径追加 | `image_url` | 路径自动追加 `/image-to-image` |
| **MiniMax** | subject_reference 角色参考 | `subject_reference[].image_file` | 不切换 |
| **SiliconFlow** | 自动切换 Edit 模型 | `image` | `Qwen/Qwen-Image` → `Qwen/Qwen-Image-Edit` |
| **Tongyi** | 切换 endpoint + 模型 | `input.base_image_url` | `wanx-v1` → `wanx2.1-imageedit`，endpoint 从 `text2image` → `image2image` |

---

## 工具入口流程

### `tool_image_generate(args)` 主流程

```mermaid
flowchart TD
    START(["tool_image_generate(args)"]) --> LOAD["加载配置<br/>provider::load_store().image_generate"]
    LOAD --> ACTION{"action 参数?"}

    ACTION -- "list" --> LIST["build_list_result()<br/>返回所有 Provider 能力列表"]
    LIST --> RETURN_LIST(["返回结果"])

    ACTION -- "generate" --> PARSE["解析参数<br/>prompt / size / n / model<br/>aspectRatio / resolution<br/>image / images"]
    PARSE --> LOAD_IMG["加载参考图<br/>合并 image + images → 去重<br/>load_input_image() → Vec&lt;InputImage&gt;"]
    LOAD_IMG --> INFER{"有参考图且<br/>无显式 resolution?"}
    INFER -- "是" --> AUTO_RES["infer_resolution()<br/>自动推断 1K/2K/4K"]
    INFER -- "否" --> BUILD_CAND
    AUTO_RES --> BUILD_CAND

    BUILD_CAND{"model 参数?"}
    BUILD_CAND -- "指定模型" --> SINGLE["单一 Provider<br/>无 failover"]
    BUILD_CAND -- "auto" --> ALL["所有已启用 Provider<br/>按配置顺序"]
    SINGLE --> LOOP
    ALL --> LOOP

    LOOP["Failover 循环"] --> RESOLVE["resolve_provider(id)<br/>→ Box&lt;dyn ImageGenProviderImpl&gt;"]
    RESOLVE --> VALIDATE{"validate_capabilities()<br/>能力兼容?"}
    VALIDATE -- "不兼容" --> SKIP["跳过，记入 failover_log"]
    SKIP --> NEXT_P{"还有候选?"}
    NEXT_P -- "是" --> RESOLVE
    NEXT_P -- "否" --> FAIL(["返回聚合错误"])

    VALIDATE -- "兼容" --> GEN["构造 ImageGenParams<br/>generate(params).await"]
    GEN --> RESULT{"结果?"}
    RESULT -- "Ok" --> SUCCESS["build_success_result()<br/>保存图片 → __MEDIA_URLS__<br/>→ failover 日志 → 结构化日志"]
    SUCCESS --> RETURN_OK(["返回成功"])

    RESULT -- "Err" --> CLASSIFY{"classify_error()<br/>可重试?"}
    CLASSIFY -- "是 + attempt < 1" --> RETRY["等待 2-10s<br/>指数退避重试"]
    RETRY --> GEN
    CLASSIFY -- "否" --> LOG_ERR["记入 failover_log"]
    LOG_ERR --> NEXT_P

    style START fill:#e0e7ff,stroke:#4f46e5
    style LIST fill:#d1fae5,stroke:#059669
    style SUCCESS fill:#d1fae5,stroke:#059669
    style FAIL fill:#fee2e2,stroke:#dc2626
    style RETURN_LIST fill:#d1fae5,stroke:#059669
    style RETURN_OK fill:#d1fae5,stroke:#059669
```

### 参考图加载流程

```mermaid
flowchart LR
    INPUT["path_or_url"] --> CHECK{"来源类型?"}
    CHECK -- "data:..." --> DECODE["base64 解码"]
    CHECK -- "https://..." --> DL["reqwest 下载<br/>30s 超时"]
    CHECK -- "~/path<br/>/path<br/>file://..." --> READ["tokio::fs::read<br/>+ MIME 推断"]

    DECODE --> IMG["InputImage<br/>{ data, mime }"]
    DL --> IMG
    READ --> IMG

    style INPUT fill:#e0e7ff,stroke:#4f46e5
    style IMG fill:#d1fae5,stroke:#059669
```

### Resolution 自动推断

```mermaid
flowchart LR
    IMGS["InputImage[]"] --> DECODE2["image crate 解码<br/>取 width × height"]
    DECODE2 --> MAX["max_dim = max(w, h)"]
    MAX --> C1{"≥ 3000?"}
    C1 -- "是" --> R4K["4K"]
    C1 -- "否" --> C2{"≥ 1500?"}
    C2 -- "是" --> R2K["2K"]
    C2 -- "否" --> R1K["1K"]

    style R4K fill:#fecaca,stroke:#dc2626
    style R2K fill:#fef3c7,stroke:#d97706
    style R1K fill:#d1fae5,stroke:#059669
```

---

## 动态工具描述

`get_image_generate_tool_dynamic(config)` 在每次注入工具时调用，根据当前已启用的 Provider **动态生成**工具的 JSON Schema 描述：

```mermaid
flowchart LR
    CONFIG["ImageGenConfig<br/>已启用的 Providers"] --> SCAN["扫描 capabilities()"]

    SCAN --> E["edit_providers<br/>支持编辑的 Provider<br/>含 max_input_images"]
    SCAN --> M["multi_image_providers<br/>支持多图编辑"]
    SCAN --> A["ar_providers<br/>支持 aspectRatio"]
    SCAN --> R["res_providers<br/>支持 resolution"]
    SCAN --> N["max_n<br/>最大 count"]

    E --> DESC["动态生成 Tool JSON Schema"]
    M --> DESC
    A --> DESC
    R --> DESC
    N --> DESC

    DESC --> D1["description: 编辑支持列表"]
    DESC --> D2["image: 支持编辑的 Provider"]
    DESC --> D3["images: 支持多图的 Provider"]
    DESC --> D4["aspectRatio: 支持的 Provider"]
    DESC --> D5["resolution: 支持的 Provider"]
    DESC --> D6["n.maximum: 动态上限"]

    style CONFIG fill:#e0e7ff,stroke:#4f46e5
    style DESC fill:#fef3c7,stroke:#d97706
```

新增或移除 Provider 时，工具描述自动更新，无需手动维护。

---

## 降级与重试策略

```mermaid
flowchart TD
    ERR(["生成失败"]) --> CAP{"Capabilities<br/>兼容?"}
    CAP -- "不兼容" --> SKIP1["跳过 Provider<br/>记入 failover_log"]
    CAP -- "兼容" --> CLASSIFY{"classify_error()"}

    CLASSIFY -- "Timeout<br/>Overloaded<br/>RateLimit" --> RETRY{"attempt < 1?"}
    RETRY -- "是" --> BACKOFF["指数退避 2-10s<br/>重试 1 次"]
    RETRY -- "否" --> NEXT["记入 failover_log<br/>→ 下一个 Provider"]

    CLASSIFY -- "Auth<br/>Billing<br/>ModelNotFound" --> NEXT
    CLASSIFY -- "Unknown<br/>ContextOverflow" --> NEXT

    SKIP1 --> NEXT
    NEXT --> HAS{"还有候选<br/>Provider?"}
    HAS -- "是" --> LOOP(["尝试下一个"])
    HAS -- "否" --> FAIL(["返回聚合错误<br/>failover_log 透明展示"])

    style ERR fill:#fee2e2,stroke:#dc2626
    style BACKOFF fill:#fef3c7,stroke:#d97706
    style FAIL fill:#fee2e2,stroke:#dc2626
```

---

## Provider 特殊机制

### Google: Size → AspectRatio 映射

Google API 不直接接受像素尺寸，通过 `imageConfig` 传递：

```mermaid
flowchart LR
    subgraph "size → aspectRatio"
        S1["1024x1024"] --> A1["1:1"]
        S2["1024x1536"] --> A2["2:3"]
        S3["1536x1024"] --> A3["3:2"]
        S4["1024x1792"] --> A4["9:16"]
        S5["1792x1024"] --> A5["16:9"]
    end
    subgraph "resolution → imageSize"
        R1["1K"] --> I1["不传（默认）"]
        R2["2K"] --> I2["imageSize: 2K"]
        R3["4K"] --> I3["imageSize: 4K"]
    end
```

### Google: ThinkingLevel

通过 `ImageGenProviderEntry.thinking_level` 配置（`"MINIMAL"` 或 `"HIGH"`），控制 Gemini 在图片生成时的推理深度。注入到 `generationConfig.thinkingConfig.thinkingLevel`。

### Fal: AspectRatio 枚举映射

Fal API 使用枚举而非比例字符串：

```mermaid
flowchart LR
    subgraph "aspectRatio → Fal 枚举"
        AR1["1:1"] --> E1["square_hd"]
        AR2["4:3"] --> E2["landscape_4_3"]
        AR3["3:4"] --> E3["portrait_4_3"]
        AR4["16:9"] --> E4["landscape_16_9"]
        AR5["9:16"] --> E5["portrait_16_9"]
    end

    subgraph "aspectRatio + resolution → 像素"
        direction TB
        COMBO["aspectRatio + resolution"] --> CALC["长边 = edge 像素<br/>短边 = 按比例缩放"]
        RES["1K→1024 / 2K→2048 / 4K→4096"] --> CALC
    end
```

### Fal: 编辑路径自动追加

有参考图时，自动在 model 路径后追加 `/image-to-image`：

```
"fal-ai/flux/dev" → "fal-ai/flux/dev/image-to-image"
```

已包含 `/image-to-image` 或 `/edit` 后缀的路径不重复追加。

### SiliconFlow: 自动模型切换

有参考图时自动切换模型：
- 文生图：`Qwen/Qwen-Image`（默认 50 步推理）
- 图片编辑：`Qwen/Qwen-Image-Edit`（20 步推理 + guidance_scale=7.5）

### Tongyi: 异步轮询架构

通义万相采用异步任务模式，是唯一需要轮询的 Provider：

```mermaid
sequenceDiagram
    participant T as tool_image_generate
    participant D as DashScope API
    participant CDN as Image CDN

    T->>D: POST /aigc/text2image/image-synthesis<br/>Header: X-DashScope-Async: enable
    D-->>T: { task_id, task_status: "PENDING" }

    loop 轮询（1s → 2s → 3s，上限 3s）
        T->>D: GET /api/v1/tasks/{task_id}
        D-->>T: { task_status: "RUNNING" }
    end

    T->>D: GET /api/v1/tasks/{task_id}
    D-->>T: { task_status: "SUCCEEDED", results: [{ url }] }

    T->>CDN: GET image_url
    CDN-->>T: image bytes
```

- 文生图 endpoint：`/api/v1/services/aigc/text2image/image-synthesis`
- 编辑 endpoint：`/api/v1/services/aigc/image2image/image-synthesis`（模型切换为 `wanx2.1-imageedit`，`function: "description_edit"`）
- 超时：`timeout_secs`（默认 60s）

### Tongyi: Size 格式转换

通义 API 使用 `*` 分隔尺寸（如 `1024*1024`），而非标准的 `x`：

```rust
fn convert_size_format(size: &str) -> String {
    size.replace('x', "*")
}
```

---

## 持久化配置

### 配置结构

```rust
pub struct ImageGenConfig {
    pub providers: Vec<ImageGenProviderEntry>,  // 有序列表（顺序 = 优先级）
    pub timeout_seconds: u64,                    // 默认 60
    pub default_size: String,                    // 默认 "1024x1024"
}

pub struct ImageGenProviderEntry {
    pub id: String,                    // "openai", "google", ...
    pub enabled: bool,
    pub api_key: Option<String>,
    pub base_url: Option<String>,      // 自定义 API 地址
    pub model: Option<String>,         // 自定义模型名
    pub thinking_level: Option<String>,// Google 专用
}
```

存储位置：`~/.opencomputer/config.json` 的 `imageGenerate` 字段。

### 配置自动补齐

`backfill_providers()` 在每次加载配置时调用：
1. 规范化已有 Provider ID（向后兼容：`"OpenAI"` → `"openai"`，`"MiniMax"` → `"minimax"` 等）
2. 检查 `known_provider_ids()` 列表，补齐缺失的 Provider（disabled 状态）

新增 Provider 后，用户已有的 `config.json` 会自动补齐新条目，无需迁移。

---

## 前端设置面板

### 组件：`ImageGeneratePanel.tsx`

```mermaid
block-beta
    columns 1
    block:panel["ImageGeneratePanel"]
        columns 1
        header["图片生成 — 配置 AI 图片生成服务商"]
        block:providers["服务商（排在前面的优先使用）"]
            columns 1
            p1["[1] OpenAI — 启用/API Key/Base URL/Model/测试连接 — ↑↓"]
            p2["[2] Google — 启用/API Key/Model 下拉/ThinkingLevel — ↑↓"]
            p3["[3] Fal — 启用/API Key/Base URL/Model — ↑↓"]
            p4["[4] MiniMax — 启用/API Key/Base URL/Model — ↑↓"]
            p5["[5] SiliconFlow — 启用/API Key/Base URL/Model — ↑↓"]
            p6["[6] ZhipuAI — 启用/API Key/Base URL/Model — ↑↓"]
            p7["[7] Tongyi — 启用/API Key/Base URL/Model — ↑↓"]
        end
        block:global["全局设置"]
            columns 3
            size["默认尺寸 ▾"]
            timeout["超时（秒）"]
            save["保存按钮（三态）"]
        end
    end

    style header fill:#e0e7ff,stroke:#4f46e5
    style providers fill:#f0fdf4,stroke:#22c55e
    style global fill:#fefce8,stroke:#ca8a04
```

- **优先级排序**：上下箭头调整顺序，顺序即 failover 优先级
- **测试连接**：调用 `test_image_generate` 命令验证 API Key 可用性
- **Google 模型选择器**：内置 6 个预设模型 + 自定义模式
- **三态保存按钮**：saving（旋转动画）→ saved（绿色 ✓）→ idle

### Tauri 命令

| 命令 | 功能 |
|------|------|
| `get_image_generate_config` | 加载配置（含 backfill） |
| `save_image_generate_config` | 保存配置到 config.json |
| `test_image_generate(provider_id, api_key, base_url)` | 测试 Provider 连通性 |

---

## 文件结构

```mermaid
graph LR
    subgraph backend["crates/oc-core/src/tools/image_generate/"]
        MOD["mod.rs<br/>核心模块：trait + capabilities<br/>+ 入口 + failover + list"]
        OAI["openai.rs<br/>gpt-image-1<br/>base64 响应"]
        GOO["google.rs<br/>Gemini<br/>多模态 + thinkingLevel"]
        FAL["fal.rs<br/>Flux<br/>URL + CDN 下载"]
        MM["minimax.rs<br/>image-01<br/>subject_reference"]
        SF["siliconflow.rs<br/>Qwen-Image<br/>自动切 Edit 模型"]
        ZP["zhipu.rs<br/>CogView-4<br/>中文文字渲染"]
        TY["tongyi.rs<br/>wanx-v1<br/>异步轮询 + 编辑"]
    end

    subgraph infra["基础设施"]
        DEF["definitions.rs<br/>动态 JSON Schema"]
        PRO["commands/provider.rs<br/>test_image_generate"]
        CFG["commands/config.rs<br/>get/save config"]
    end

    subgraph frontend["前端"]
        PANEL["ImageGeneratePanel.tsx<br/>设置面板"]
        I18N["i18n/locales/{en,zh}.json<br/>国际化"]
    end

    MOD --> OAI & GOO & FAL & MM & SF & ZP & TY
    DEF -.-> MOD
    PANEL -.-> CFG
    PANEL -.-> PRO

    style backend fill:#f0fdf4,stroke:#22c55e
    style infra fill:#fef3c7,stroke:#d97706
    style frontend fill:#e0e7ff,stroke:#4f46e5
```

---

## 扩展新 Provider 指南

新增一个 Provider 只需 4 步：

### 1. 新建 Provider 文件

`crates/oc-core/src/tools/image_generate/{provider_id}.rs`，参照 `minimax.rs` 模板：

```rust
pub(crate) struct MyProvider;

impl ImageGenProviderImpl for MyProvider {
    fn id(&self) -> &str { "myprovider" }
    fn display_name(&self) -> &str { "MyProvider" }
    fn default_model(&self) -> &str { "model-v1" }
    fn capabilities(&self) -> ImageGenCapabilities { /* 声明能力 */ }
    fn generate<'a>(&'a self, params: ImageGenParams<'a>)
        -> Pin<Box<dyn Future<Output = Result<ImageGenResult>> + Send + 'a>> {
        Box::pin(generate_impl(params))
    }
}

async fn generate_impl(params: ImageGenParams<'_>) -> Result<ImageGenResult> {
    // 1. 解析 base_url（默认值 + 用户自定义覆盖）
    // 2. 构建请求体（根据 params.input_images 判断生成/编辑模式）
    // 3. 日志记录
    // 4. 发送 HTTP 请求（使用 crate::provider::apply_proxy 代理）
    // 5. 解析响应 → Vec<GeneratedImage>
    // 6. 返回 ImageGenResult
}
```

### 2. 注册到 mod.rs

```rust
pub(crate) mod myprovider;

// resolve_provider() 加分支
"myprovider" => Some(Box::new(myprovider::MyProvider)),

// known_provider_ids() 加 id
&[..., "myprovider"]

// normalize_provider_id() 加映射
"MyProvider" => "myprovider".to_string(),

// default_providers() 加条目
ImageGenProviderEntry { id: "myprovider".to_string(), ..Default::default() },
```

### 3. 前端面板 + test 命令

- `ImageGeneratePanel.tsx` 的 `PROVIDER_DISPLAY` 和 `DEFAULT_CONFIG` 加条目
- `provider.rs` 的 `test_image_generate()` 加 test 分支
- `en.json` / `zh.json` 加 i18n key

### 4. 文档

- `CHANGELOG.md` 记录
- `CLAUDE.md` / `AGENTS.md` / `.agent/rules/default.md` 更新 Provider 数量

**无需修改**：`tool_image_generate()`、`build_success_result()`、`validate_capabilities()`、`get_image_generate_tool_dynamic()`——这些函数通过 trait 和 capabilities 自动适配新 Provider。
