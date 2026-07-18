# 设计空间（Design Space）子系统架构文档

> 返回 [文档索引](../README.md)
>
> 设计空间是 Hope Agent 的 **agent 原生设计工作空间**：用户与模型协作，从一句话或参考图产出**自包含、可交付的设计产物**（网页 / 移动原型 / 演示文稿 / 仪表盘 / 海报 / 文档 / 邮件 / 图像 / 动效 / 音频 / 交互组件），以可复用的**品牌设计系统**为底座，在沙盒面板实时预览、可视化直接微调、版本管理、一键导出，并可经**工程轴**把设计系统一路交付到代码（多平台 Token 导出 / Figma 导入 / 代码交付包 / 绑定代码工程同步），与[知识空间](knowledge-base.md)、[项目](project.md)深度联动。侧边栏入口紧贴「知识空间」下方。
>
> 产品名 **设计空间**；代码标识 `design`（模块 `crates/ha-core/src/design/`、agent 工具 `design`、数据库 `design.db`、前端视图 `DesignView`、右侧面板与 i18n 命名空间 `design`）。产品名与代码标识**均不引用任何外部参考实现的名称**（品牌产品名仅作设计数据出现在品牌参考系统里，见 §6.3）。
>
> 本文是子系统设计与实现的单一真相源；跨 PR 必守的红线摘要另见 [AGENTS.md](../../AGENTS.md)。

## 目录

1. [定位与设计原则](#1-定位与设计原则)
2. [核心竞争力：四大差异化](#2-核心竞争力四大差异化)
3. [系统架构总览](#3-系统架构总览)
4. [核心概念与数据模型](#4-核心概念与数据模型)
5. [渲染管线（轻量自包含 HTML）](#5-渲染管线轻量自包含-html)
6. [设计系统层（品牌契约 + Token 编译）](#6-设计系统层品牌契约--token-编译)
7. [可视化直接微调（选中→反查→回写）](#7-可视化直接微调选中反查回写)
8. [Agent 工具面（`design` 工具）](#8-agent-工具面design-工具)
9. [前端视图与工作台](#9-前端视图与工作台)
10. [导出与产物库](#10-导出与产物库)
11. [质量评审门与设计方向选择器](#11-质量评审门与设计方向选择器)
12. [与现有子系统的契约](#12-与现有子系统的契约)
13. [权限 · 安全 · 沙箱 · 无痕（红线）](#13-权限--安全--沙箱--无痕红线)
14. [配置（设置三件套）](#14-配置设置三件套)
15. [HTTP 路由与 Tauri 命令对照](#15-http-路由与-tauri-命令对照)
16. [文件清单（注册触点）](#16-文件清单注册触点)
17. [命名与关键设计决策](#17-命名与关键设计决策)

---

## 1. 定位与设计原则

### 1.1 一句话定位

设计空间让模型与用户协作，从一句话或参考图产出**成体系、可交付的设计产物**，落在一个稳定、快速、可视化可编辑的工作台里，一键导出与沉淀，并经**工程轴**把设计系统一路推到代码交付边界（多平台 Token 导出 / Figma 导入 / 代码交付包 / 绑定代码工程同步，见 [§6.7](#67-多平台-token-导出designtokenexport)–§6.8、[§10](#10-导出与产物库)）。它对标 agent 原生设计工作空间这一品类，覆盖其全部产物形态与设计系统机制、并向工程交付延伸，在四个方向上做出超越（见 [§2](#2-核心竞争力四大差异化)）。

### 1.2 设计原则（每一条都直接回应旧版设计工坊的失败点）

新版是对既有 `feat/atelier` 分支的**推倒重做**。旧版用户验收暴露三个核心痛点：**画布交互卡顿不稳、渲染重且易白屏、可视化微调不好用**。以下原则逐条对症。

1. **轻量自包含产物，拒绝浏览器内编译（对症"渲染重/白屏"）**：每个产物是一份**自包含 HTML**（内联 CSS/JS，依赖走 vendored 本地资产，默认零网络）。由模型直接生成、iframe 直接加载渲染——**绝不在浏览器里编译 React/JSX/Tailwind**，无 `esbuild-wasm` 冷启动、无运行时打包、无白屏看门狗。这也让产物天然可导出、可分享、可 diff。
2. **产物为中心的稳定工作台，拒绝脆弱无限画布（对症"画布卡/不稳"）**：主编辑面是**单产物聚焦预览**（一个稳定 iframe + fit/百分比缩放下拉，纯 CSS 缩放，无自研 transform）；多产物概览用**纯 CSS grid 缩略图墙**（无平移 / 无自研缩放 / 无 pointer capture 逻辑）。从架构上根除卡顿与指针捕获泄漏类 bug。
3. **可视化微调建立在纯 HTML 的确定性映射之上（对症"微调不好用"）**：产物是纯 HTML，渲染 DOM ≈ 源码结构，因此"选中元素→改属性→回写源码"是**确定性字节范围 patch**（渲染期注入稳定 `data-ds-oid`，回写走单一命中 + `expected` stale-write 守卫 + 撤销/重做）。旧版败在 JSX→React→DOM 的有损编译映射上，本版从源头绕开。
4. **文件即真相源**：产物（`index.html` + 版本快照）与设计系统（`DESIGN.md` + `tokens.json`）都是磁盘上的真实文件；`design.db` 是**可重建的元数据注册表 / 索引**（删了能从磁盘全量重建）。对齐 [知识空间 D9](knowledge-base.md) 与 [项目](project.md) 既有红线。
5. **核心逻辑全进 ha-core**（零 Tauri 依赖）：业务、渲染编排、token 编译、oid 回写、索引全在 `crates/ha-core/src/design/`；`src-tauri` / `ha-server` 只做薄壳。
6. **Transport 双实现**：每个新 invoke 同时实现 Tauri + HTTP（见 [transport-modes.md](transport-modes.md)）。
7. **设置三件套**：新增用户可调字段必须同 PR 具备 GUI 控件 + `ha-settings` 分支 + SKILL.md 登记（见 [AGENTS.md 设置约定](../../AGENTS.md)）。
8. **安全等价于 Canvas**：iframe `sandbox="allow-scripts"`（无 same-origin）、静态托管三道闸、`eval`/脚本只在沙盒内、写盘走 `write_atomic` + 作用域闭合、出站走 `security::ssrf`。
9. **原创原型语言 + 品牌风格参考（附免责声明）**：内置设计系统两类——6 套**原创原型化**设计语言（极简现代 / 编辑杂志 / 科技暗色 …）+ 一批**品牌风格参考**（对各品牌公开视觉语言的独立再诠释；`build_system_md` 对 `brand_ref` 渲染时**必附免责声明**、非官方、无隶属 / 授权，详见 §6.3）。**红线**：代码 / 注释 / commit / 文档 / UI / i18n 不出现任何**外部参考实现**（Hope Agent 对标的开源设计项目）的名字；品牌产品名仅作**设计数据**出现在品牌参考系统里。（注：早期「不克隆真实品牌」的立场已调整为「独立再诠释 + 免责声明」，理由与落地见 §6.3。）

### 1.3 与旧版设计工坊、与 Canvas 的关系

- **不复用、不依赖 `feat/atelier`**：新版从零构建，独立模块 `design/`、独立表 `design.db`、独立视图 `DesignView`。atelier 的重型离线 React 运行时、无限画布、esbuild-wasm 管线**一律不引入**。
- **与 [Canvas](canvas.md) 分工**：Canvas 是对话内随手出图的轻量沙盒（7 种 content_type、CDN 脚本、易逝）；设计空间是可管理、可交付、可微调的成体系工作空间。二者共存、不混、各自独立事件流。设计空间**不复用 canvas 的表 / 工具 / 面板**，但借鉴其已验证的沙盒静态托管三闸与 `resolveAssetUrl` Tauri/HTTP 分流思路。

---

## 2. 核心竞争力：四大差异化

用户拍板：这四个方向**全做，且要好用 + 完美**。它们是架构重点投入区，贯穿数据模型、工具面与 UI。

### D1 · 可视化直接微调（做扎实）

选中产物内任意元素 → 检视面板改文案 / 配色 / 间距 / 字号 / 尺寸 → **即时预览 + 回写源码**。工程做法见 [§7](#7-可视化直接微调选中反查回写)。**做扎实的关键**：产物是纯 HTML，渲染期注入稳定 `data-ds-oid`，`oid → 源码字节范围`一一对应，回写确定性、可撤销、有 stale-write 守卫；单一稳定 iframe，无画布 transform 干扰。这是旧版做不好、本版从架构层解决的能力。

### D2 · 更强的品牌设计系统（本地护城河）

一键从**截图 / 图片 / URL / 现有本地代码工程**反向提取品牌设计契约（`DESIGN.md` 9 段 + `tokens.json`），并可视化管理、跨产物套用、跨会话/项目全局引用。因 Hope Agent 是本地桌面 Agent（有文件系统 / exec / 多模态），"读本地工程提取设计系统"是外部云端产品做不到的护城河。详见 [§6](#6-设计系统层品牌契约--token-编译)。

### D3 · 一键导出与产物库

统一产物库（缩略图墙 + 版本对比 + 批量操作），一键导出 **HTML / PDF / PPTX / PNG**，保真优先。详见 [§10](#10-导出与产物库)。

### D4 · 与知识空间 / 项目联动

设计产物可**沉淀进知识空间**（生成一条 KB 笔记内嵌预览与元数据，进入第二大脑可检索）；设计系统可被 agent **全局引用**（作为可复用上下文注入 system prompt，像记忆/知识那样约束生成）；设计项目可**绑定 Hope Agent 项目**（共享工作目录）。详见 [§12](#12-与现有子系统的契约)。

---

## 3. 系统架构总览

```mermaid
graph TD
    subgraph 前端
        VIEW["DesignView<br/>（侧边栏独立视图）"]
        HOME["DesignHome<br/>prompt-first + 类型卡 + 最近项目墙"]
        STUDIO["DesignStudio<br/>产物库 + 单产物聚焦预览 + 检视抽屉 + AI 面板"]
        INSP["DesignInspector<br/>选中元素 → 属性编辑"]
    end

    subgraph Transport
        TX["getTransport()<br/>Tauri invoke / HTTP COMMAND_MAP"]
    end

    subgraph ha-core（零 Tauri 依赖）
        TOOL["tools/design/<br/>agent 工具 design（多 action）"]
        SVC["design/service.rs<br/>owner 平面业务入口"]
        RENDER["design/renderer.rs<br/>自包含 HTML 编译 + inspector bridge 注入"]
        SYS["design/system.rs<br/>DESIGN.md 解析 → tokens.json → :root CSS 变量"]
        PATCH["design/patch.rs<br/>oid → 源码字节范围 确定性回写"]
        CRIT["design/critique.rs<br/>5 维质量门（side_query）"]
        EXPORT["design/export.rs<br/>HTML/PDF/PPTX/PNG"]
        DB[("design.db<br/>元数据注册表/索引")]
        FILES[("~/.hope-agent/design/<br/>systems/ + projects/ 真实文件")]
        BUS["EventBus（design:*）"]
    end

    VIEW --> HOME & STUDIO
    STUDIO --> INSP
    VIEW <--> TX
    TX <--> SVC
    TOOL --> SVC
    SVC --> RENDER & SYS & PATCH & CRIT & EXPORT
    RENDER --> FILES
    SYS --> FILES
    SVC --> DB
    SVC --> BUS
    BUS -- "design:artifact_ready / design:reload / ..." --> STUDIO
    STUDIO -- "iframe src" --> FILES
    STUDIO <-. "postMessage（select / edit / snapshot）" .-> IFRAME["产物 iframe<br/>（inspector bridge）"]
```

**两条鉴权平面（物理隔离，对齐知识空间 D10 / canvas owner 面）：**

- **owner 平面**（Tauri / HTTP，`service.rs`）：本机 / API key 信任，负责 UI 的项目/产物/系统 CRUD、可视化编辑回写、导出——**不经 agent 访问检查**。
- **agent 平面**（`design` 工具）：模型侧生成与操作走工具，受权限引擎与无痕/访问约束裁决。

---

## 4. 核心概念与数据模型

### 4.1 概念

| 概念 | 定义 | 生命周期 |
| --- | --- | --- |
| **设计项目（Project）** | 顶层容器，聚合一组产物，可选绑定一个默认设计系统与一个 Hope Agent 项目 | 用户/模型创建 → 增删产物 → 删除级联清目录 |
| **产物（Artifact）** | 单个可交付设计，有 `kind`（web/mobile/deck/dashboard/poster/document/email/image），对应磁盘一个目录 + 一份自包含 `index.html` | `create` → `update`（累加版本）→ `delete` |
| **产物版本（Version）** | 一次 update / restore / 可视化编辑产生的源码快照，带 `origin`（`ai` 生成/精修 / `manual` 可视化微调·换系统 / `restore` 回滚） | 递增；超 `maxVersionsPerArtifact` 时**里程碑感知淘汰**：优先删最旧的 `manual`（微调自动保存），保留 `ai`/`restore` 里程碑与当前（最新）版本——防重度微调把 AI 里程碑挤掉（`cleanup_old_versions`，manual 淘尽仍超限才动最旧 ai/restore） |
| **设计系统（DesignSystem）** | 可复用品牌契约：`DESIGN.md`（9 段，真相源）+ `tokens.json`（解析缓存） | 内置只读 / 用户创建 / 反向提取；owner 平面可**改名（内置拒改）/ 删除**；套用到产物即注入 `:root` token |
| **设计模板（Recipe）** | 某产物形态的生成模板（`RECIPE.md`：frontmatter + 生成指令 + 预览），供模型 `list_recipes/get_recipe` 参考 | 内置随 App 发行 + 用户自建（managed 目录） |
| **oid 映射（oidmap）** | 渲染期为源码每个元素分配的稳定 `data-ds-oid → 源码字节范围`，可视化回写用 | 每次渲染重算；随版本落盘 |

### 4.2 存储布局（磁盘 = 内容真相源）

```
~/.hope-agent/design/
├── design.db                            # SQLite（WAL + foreign_keys）：元数据注册表 / 可重建索引
├── systems/
│   └── {system-id}/
│       ├── DESIGN.md                    # 品牌契约（9 段，真相源）
│       ├── tokens.json                  # DESIGN.md 解析出的 token（可重建缓存）
│       └── assets/                      # 可选：logo / 字体引用
└── projects/
    └── {project-id}/
        ├── project.json                 # 项目元数据（真相源镜像）
        └── artifacts/
            └── {artifact-id}/
                ├── artifact.json        # 产物元数据（kind / title / system_ref / current_version）
                ├── index.html           # 当前渲染产物（自包含，真相源）
                ├── source/              # 可编辑源（body.html / style.css / script.js / data.json，按需拆分）
                ├── oidmap.json          # 当前版本 oid → 源码坐标
                ├── versions/{n}/        # 版本快照（index.html + source/ + oidmap.json）
                ├── thumbnail.jpg        # 缩略图（owner 端 JPEG 强校验后落盘）
                └── exports/             # 导出物（**必须 gitignore**；restore 会清）
```

内置设计系统与模板随 App 发行，源在仓库 `design-assets/`（`systems/` + `recipes/`），首启复制/懒加载到 managed 目录，用户可覆盖（优先级：project > managed > bundled，对齐技能来源模型）。

路径解析集中在 [`paths.rs`](../../crates/ha-core/src/paths.rs)：`design_dir` / `design_systems_dir` / `design_projects_dir` / `design_project_dir(id)` / `design_artifact_dir(project_id, artifact_id)`。

### 4.3 SQLite 表（`design.db`，元数据注册表）

```sql
CREATE TABLE design_projects (
    id TEXT PRIMARY KEY,               -- UUID v4
    title TEXT NOT NULL,
    description TEXT,
    color TEXT,                        -- 可选主题色
    default_system_id TEXT,            -- 弱引用设计系统
    ha_project_id TEXT,                -- 代码仓库绑定源之二：HA 项目（目录实时派生，§6.9）
    session_id TEXT, agent_id TEXT,    -- 弱引用来源（无 FK）
    created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
    metadata TEXT,                     -- 预留 JSON
    code_dir TEXT                      -- 代码仓库绑定源之一：本机目录（canonical，§6.9，与 ha_project_id 互斥）
);

CREATE TABLE design_artifacts (
    id TEXT PRIMARY KEY,               -- UUID v4
    project_id TEXT NOT NULL REFERENCES design_projects(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    kind TEXT NOT NULL,                -- web|mobile|deck|dashboard|poster|document|email|image
    system_id TEXT,                    -- 可选：覆盖项目默认设计系统
    status TEXT NOT NULL DEFAULT 'ready', -- planned|generating|ready|failed
    viewport_w INTEGER, viewport_h INTEGER,
    current_version INTEGER DEFAULT 1,
    critique_score REAL,               -- 最近一次质量门总分（可空）
    thumbnail_path TEXT,
    position INTEGER,                  -- 页面墙排序（list ORDER BY position ASC）
    folder TEXT NOT NULL DEFAULT '',   -- 归属文件夹斜杠路径，空 = 根（path-based，无 folder 表）
    created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
    metadata TEXT
);

CREATE TABLE design_folders (           -- 仅记录空文件夹（有产物的文件夹从 artifacts.folder 派生）
    project_id TEXT NOT NULL REFERENCES design_projects(id) ON DELETE CASCADE,
    path TEXT NOT NULL,               -- 斜杠路径
    created_at TEXT NOT NULL,
    PRIMARY KEY(project_id, path)
);

CREATE TABLE design_artifact_versions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    artifact_id TEXT NOT NULL REFERENCES design_artifacts(id) ON DELETE CASCADE,
    version_number INTEGER NOT NULL,
    message TEXT,                      -- create/update/restore/visual-edit 说明
    critique_score REAL,
    created_at TEXT NOT NULL,
    UNIQUE(artifact_id, version_number)
);

CREATE TABLE design_systems (           -- DESIGN.md 的可重建索引
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL, slug TEXT NOT NULL,
    source TEXT NOT NULL,              -- builtin|user|extracted
    summary TEXT,                     -- 主题气质一句话
    thumbnail_path TEXT,
    created_at TEXT NOT NULL, updated_at TEXT NOT NULL
);

CREATE INDEX idx_design_artifacts_project ON design_artifacts(project_id, updated_at DESC);
CREATE INDEX idx_design_versions_artifact ON design_artifact_versions(artifact_id, version_number DESC);
CREATE INDEX idx_design_projects_session  ON design_projects(session_id, updated_at DESC);
```

**设计要点：**

- 表是**元数据注册表**，产物正文（`index.html` / `source/`）与设计系统正文（`DESIGN.md`）在磁盘。`reindex` 可从磁盘全量重建 DB（对齐知识空间"索引可重建"红线）。
- `session_id` / `agent_id` / `ha_project_id` 均弱引用无 FK：删会话不级联删设计（跨会话复用价值）；删 Hope Agent 项目由 owner 侧显式处理。
- 版本快照式（非 diff）：换存储简单与 restore 可靠；`current_version` 是逻辑游标，prune 旧版本不影响它。

---

## 5. 渲染管线（轻量自包含 HTML）

**核心分水岭：产物是模型直接产出的自包含 HTML，Rust 端只做"包裹 + token 注入 + bridge 注入"，绝不做编译。** 前端 iframe 直接加载 `index.html`，启动即渲染。

### 5.1 编译入口

`renderer::build_artifact_html(kind, system_tokens, parts) -> String`：

1. **骨架包裹**：按 `kind` 选骨架（`web`/`document` 普通文档、`mobile` 设备框、`deck` 自带极简翻页器、`dashboard`/`poster`/`email` 对应视口容器）。
2. **Token 注入**：把设计系统 `tokens.json` 展开为 `:root { --ds-color-*, --ds-space-*, --ds-font-* ... }`，产物 CSS 一律引用这些变量 → 换系统即换皮，保证一致性（约束优先于自由）。
3. **用户源注入**：`parts.html`（body 结构）+ `parts.css`（内联 `<style>`）+ `parts.js`（内联 `<script>`，可选）。
4. **oid 标注**：解析 body HTML，为每个元素注入 `data-ds-oid="{n}"`（源码 DOM 顺序确定性编号），同时产出 `oidmap.json`（`oid → 源码字节范围`）。见 [§7](#7-可视化直接微调选中反查回写)。
5. **inspector bridge 注入**（仅可编辑 kind + 非导出渲染）：一段自包含脚本，负责选中高亮 / hover overlay / 文本就地编辑 / snapshot，全部通过 `postMessage` 与父窗通信（无 same-origin）。
6. **零网络**：默认不引 CDN。若产物需要图表等，走 **vendored 本地库**（内联进 HTML 或从 `design/assets` 本地托管），保持沙箱零网络与 CSP=null 红线。

### 5.2 产物形态（kind）与视口

| kind | 语义 | 默认视口 | 骨架特性 |
| --- | --- | --- | --- |
| `web` | 网页 / 落地页 / 桌面原型 | 1440×自适应 | 标准文档流 |
| `mobile` | 移动端原型 | 390×844 | 设备框 + 状态栏 |
| `deck` | 演示文稿 | 1280×720 (16:9) | 自带 `<section>` 翻页器（←/→/Space、页码、缩略图轨秒跳），一份文件多页；翻页经 deck bridge `show()` = toggle `.active` + `scrollIntoView`，**兼容切换式（一次一页）与滚动堆叠式**（AI 产物 CSS 用 `.ds-slide{display:grid;min-height:100vh}` 同特异性盖住 frame_css 的 `display:none`、slide 堆叠成长滚动页）两种产出 |
| `dashboard` | 数据仪表盘 | 1440×自适应 | 网格布局容器 |
| `poster` | 海报 / 社媒图 | 尺寸预设（1080×1080 / 1080×1920 / A4 …） | 定尺容器 |
| `document` | 文档 / 规格 / 报告 | 页宽阅读容器 | 目录 + 排版 |
| `email` | 营销邮件 | 600 宽 | table 回退兼容 |
| `image` | 图像 | —— | 复用统一[媒体生成](media-generation.md)栈，产出栅格图，不走 HTML 骨架 |

`image` 是唯一非 HTML 产物：它复用统一[媒体生成](media-generation.md)子系统，把结果落进产物目录并登记，参与同一产物库 / 版本 / 导出（导出即原图）。

### 5.3 生成过程可见（状态机）

```
planned ──→ generating ──→ ready
   └──────────────┴──────→ failed
```

产物状态（`DesignArtifact.status ∈ planned|generating|ready|failed`）是产物行上的列，产物库按此列渲染角标（`generating` 转圈 / `failed` 红色警示），经 `design:artifact_ready` / `design:reload` 触发的列表刷新增量更新——**纯 DOM 卡片翻转，不涉及任何画布 transform**（对症"卡"）。状态推进不各自发独立事件。

### 5.4 事件目录（as-built）

后端 emit 7 个 `design:*` 事件（`design/service.rs` + `tools/design/mod.rs`），前端 `DesignView` 全部订阅；HTTP/WS 模式经 `WS /ws/events` 全量透传，两运行模式一致送达。payload 字段均 camelCase。

| 事件 | 触发 | Payload | 前端反应 |
| --- | --- | --- | --- |
| `design:project_changed` | 项目增/删/改 | `{projectId}` | 首页刷新项目墙 |
| `design:artifact_ready` | 单产物创建完成 | `{projectId, artifactId, sessionId}` | 刷新产物库（增量插入） |
| `design:artifact_deleted` | delete | `{projectId, artifactId}` | 命中当前预览则清空 activeArtifact + 刷新库 |
| `design:reload` | update / restore / 可视化编辑落盘 | `{artifactId}` | 同 ID remount iframe + 重取 bodyHash（防下次微调 stale） |
| `design:show` | `show` action | `{projectId, artifactId, sessionId}` | 聚焦该产物（必要时自动进项目） |
| `design:system_changed` | 设计系统增/删/改 / 反向提取 | `{systemId}` | 刷新系统选择器 |
| `design:critiqued` | `critique` action | `{artifactId, overall}` | 刷新产物库（更新评分列） |
| `design:code_drift` | code→design 回灌 stale 标记翻转（`code_sync`） | `{projectId, artifactId, stale}` | 刷新产物库（stale 徽标）+ 命中当前产物刷预览（横幅） |

### 5.5 首屏 prompt→生成（GUI 一键生成）

对齐同品类的核心交互：**首屏输入一句话即可直接生成**，不必先建项目再逐步填。

- **后端生成入口 `create_artifact_generating`**：body 为空且带 prompt 时——`image` 走 `image::generate_image_parts`（image_generate 栈）；**其余全部形态走 `design::generate::generate_design_parts`**（brief + kind 的 recipe 指导 + 设计系统 DESIGN.md/token 接地 → 一次 side-query 生成自包含 `body_html/css/js`）。生成**失败降级空壳**（`app_warn` 不 `bail`），用户可在对话里继续细化。
- **模型调用统一走 `crate::automation`（模型统一化 + 链级 failover）**：所有 design 文本 side-task（生成 / 精修 / 提取 / 方向 / 组件 / critique）经单一入口 `design::run_design_task` → `automation::run`；真流式生成经 `automation::run_streaming`；**涉图路径（照着图做 / 首页传图 / 截图提取）走 `automation::run_vision` / `run_vision_streaming`（真多模态：模型直接看原图）**。模型来源两层：**用户在 GUI 显式选的 `model_override` 最优先（单模型、失败即报错不降级——显式选择必须被尊重，涉图静默降到非视觉必坏）**；缺省走统一 `function_models.automation` 链（→ chat 默认，涉图时由 `run_vision*` 自动跳过非视觉候选）。**design 与 `function_models.vision` 彻底解耦**（该配置回归只服务普通对话的视觉桥）；design 不自持 `critiqueModel`/`extractVisionModel` 覆盖。选择器的「上次使用」记忆落 `DesignConfig.last_model`（行为记忆，照 `default_system_id` 先例）；首页所选模型随项目创建写入 `DesignProject.default_model` 作项目对话初始模型（弱引用，会话内切换不回写）。
- **模型用量全入账（红线）**：design 每条模型入口都写 `model_usage_events`——文本 side-task（含流式 `run_streaming`）与**涉图路径（截图提取 / 照着图做，走 `run_vision`/`run_vision_streaming`）统一记 `KIND_SIDE_QUERY`**（`operation` 标签 `design.extract_vision` / `design.stream` / `design.generate` 可下钻；与 Knowledge OCR 等 `run_vision` 消费者同口径，`KIND_VISION` 回归视觉桥专属）、生图记 `KIND_IMAGE_GENERATION`、音频记 `KIND_AUDIO_GENERATION`（后二者无 token、只记调用次数 + 耗时）。owner 平面操作 session_id 留空、始终入账；incognito 无 design。
- **owner 壳同步 IO 下放 blocking 池（红线）**：`commands/design.rs`（Tauri）+ `routes/design.rs`（HTTP）里对同步 `service::*`（DesignDb SQLite / 文件 IO）与 `deploy::save_cf_config` 等的调用一律经 `ha_core::blocking::run_blocking(move || …).await`，不 inline 直调阻塞 async worker；异步 `service` 入口（生成 / 流式 / 提取）本就 `.await`，不重复包。
- **参考图 → 匹配产物（「照着这张图做」，真多模态）**：`CreateArtifactInput.reference_images`（首页 ≤5 张，优先于单张 `reference_image_b64`；非媒体形态）在 `generate_design_artifact` 里逐张经 `extract::prepare_reference_image`（大小闸 + 魔数嗅探 + `downscale_for_vision` 降采样重编码）规整成**视觉附件**，与文字要求一起经 `run_vision_streaming` 上行——**选中的视觉模型同时看全部原图**流式生成（旧「describe 成文字 brief 再生成」两阶段已退役：文字中转丢精确配色 / 布局比例 / 字体质感）。入口：首页卡片传图（`+` / 粘贴 / 拖拽，≤5 张）、项目内「从参考图生成」弹窗（单张）、品牌包（每件产物都带参考图，N 件 = N 次带图调用）。多图上限 `MAX_REFERENCE_IMAGES`（后端，按规整成功计数、坏图不占名额、每件产物 ≤5）/`MAX_HOME_REF_IMAGES`（前端上限，两常量须对齐）。图-only（无文字 brief）用固定复刻指令；prompt 尾部附 `REFERENCE_IMAGE_GUIDANCE` 复刻指引，system 用 `VISION_UNTRUSTED_SYSTEM` 信封（图内文字 = 复刻素材，绝不作指令）。前端 canvas 先降采样（≤1600px JPEG）再上传（HTTP body 不超限）；参考图规整失败**回退文本 brief / 空壳**，不阻断。区别于 §6.4 反向提取（图→设计系统 token）：这里图→**可交付产物**。
- **生成输出格式**：`<<<BODY>>> / <<<CSS>>> / <<<JS>>>` 分节定界符（抗大段 HTML 的引号/换行转义，比 JSON 稳）；`strip_fence` 按行剥 ```` ```lang ```` 围栏（不能用 `trim_matches('`')`——会漏语言标签行污染内容）；**截断检测**：合规输出必含 `<<<CSS>>>`，缺失即视作 body 段被截断 → `bail` 走降级，不静默交付半截无样式产物。
- **前端 `LaunchHome`（prompt-first 首屏）**：大标题 + 大输入框（Cmd/Ctrl+Enter 生成）+ 形态 chip + **模板快选行**（`list_design_recipes_cmd` 拉内置 recipe 目录，点选 → 填入该形态 + 场景 brief，可编辑后生成）+ 内联设计系统选择器 + **参考图（`+` 按钮多选 / 粘贴 / 拖拽收 ≤5 张，逐张缩略预览、点图放大 lightbox、X 移除单张；有图无文也可生成）** + **生成模型 chip**（通用 `ModelSelector`；传图态 `requireVision` 置灰非视觉模型；选择记 `DesignConfig.last_model`、随项目创建写 `DesignProject.default_model`）+ 生成 / 品牌包按钮。**传图瞬间当前模型不认图 → 自动切到可用视觉模型 + toast（删图不切回，模型选择粘性）；无任何视觉模型 → 传图入口拦截并提示去设置**。`generateFromHome` = 建项目 → 带 prompt（+ 参考图 + modelOverride）建产物（后端生成）→ 打开；产物创建失败**回滚删除刚建的孤儿项目**；生成中禁用最近项目磁贴防导航被完成回调劫持。
- **真实缩略图墙（`ArtifactThumb` / `ProjectThumb`）**：首屏项目卡 = 该项目最近产物的**静态设计预览**——懒挂载（`IntersectionObserver`）+ `sandbox=""`（**不跑 JS**，画廊零动画开销、性能稳）+ `ResizeObserver` 等比缩放，复用产物 `index.html` 的 asset 服务，无独立缩略图存储管线。

### 5.6 真流式生成（CSS-first + 增量渲染，比裸流更稳）

owner/GUI 生成走**真 token 流式**——边生成边成形预览，而非等整份产出。核心目标是**无 FOUC**（不先闪一屏无样式内容）+ 稳定不重挂。

- **CRUX：流式 LLM 原语**（不碰共享 `side_query`）。SSE parser 机器已在主对话循环中久经考验，只差一条「单轮 prompt → 增量 token」的入口：
  - `LlmApiAdapter::one_shot_stream`（`agent/llm_adapter.rs`）——`one_shot` 的流式姊妹方法，4 个 provider impl 复用现成 body 构造器（Anthropic/OpenAIChat/OpenAIResponses 构造后 post-process 插 `stream:true`，不改 body 构造器故 prompt-cache body-shape 单测全绿；Codex 本就 `stream:true`），喂对应 `parse_*_sse`。**parser 的 `on_delta` 收的是主循环事件信封 `{"type":"text_delta","content":…}`**，故 `one_shot_stream` 内经 `unwrap_text_delta` 解信封、**只吐裸文本**（thinking/tool 信封丢弃），与 parser 的 `collected_text` 口径一致。
  - `AssistantAgent::side_query_streaming`（`agent/side_query_stream.rs`）——与 `side_query` 平行（复用 cache-safe prefix + `execute_with_failover`），差别仅「`side_query` 丢 delta / `side_query_streaming` 转发 delta」。`on_text` 收**当前 attempt 的累积文本**（非裸 delta）：failover 重试时累加器重启，调用方据新鲜快照幂等重渲染，不跨 attempt 拼接。
- **生成输出 CSS-first**（`generate.rs`）：分节顺序改 `<<<CSS>>> → <<<BODY>>> → <<<JS>>>`——CSS 段在 `<<<BODY>>>` 一出现即完整，预览可**先把最终样式注入 iframe，再流式追加 body** = 无裸奔无重排。截断检测据此改判「必含 `<<<BODY>>>`」（缺失=CSS 段被截断 → `bail` 走降级）。`strip_trailing_partial_marker` 剥尾部未闭合 marker，**只在尾部后缀是某完整 marker 的严格前缀时才截**（正文里合法的 `<<<`——git 冲突标记 / `content:"<<<"` / ASCII art——不误截、不冻结预览）。
- **端到端数据流**：owner 入口 `generate_design_artifact`（Tauri `generate_design_artifact_cmd` / HTTP `POST /api/design/artifacts/generate`）→ ① `create_artifact_shell` 建 `status=generating` 壳（`build_stream_host_html`：CSS-first head 一次定稿 + 空 body 容器 + 常驻接收脚本 + 居中 spinner）**同步返回**，前端挂稳定 iframe；② `tokio::spawn`（`AssertUnwindSafe.catch_unwind` panic 兜底）跑 `stream_generate_artifact` → `generate::stream_design_parts`（走 `side_query_streaming`，按字节增长节流）逐帧 emit `design:generate_delta`；③ 前端独立 `useEffect` 监听 delta，按 `streamId` 重置累积、按 `seq` 丢乱序帧，经 `postToIframe` 发 `ds_stream_css`（替换 `<style id=ds-user-css>`）/ `ds_stream_body`（非空才替换 `#ds-stream-body` innerHTML，清掉内嵌 spinner）——**纯 DOM 插入不编译 JSX**（守红线①）；④ `finalize_generating_artifact`（`artifact_lock` 下单次 render+落盘+`status=ready`+建首版）emit `design:generate_done` → 前端 `refreshView` + **唯一一次受控** `previewKey++` swap 到定稿 `index.html`（editable，挂 oid + inspector bridge）。
- **流式期 `editable=false` 语义**：半流式 DOM 无法稳定算 oid、半截 `<script>` 会抛错，故壳页不标 oid / 不挂 bridge / 不跑 body 内 `<script>`（`innerHTML` 天然不执行脚本 → 流式期无副作用）；oid/bridge 仅在定稿 index.html 生效。
- **降级 / 韧性 / 安全**：生成失败（截断 / 空 body / 无后端 / panic）经 `degrade_to_placeholder` 落**干净占位** index.html（非永久 spinner）+ `status=failed`，emit `design:generate_error`；产物已删则**静默**（`degrade` 返 `false` → 不 emit，对齐 `finalize` 已删返 `None` 静默契约）。`delete_artifact` 与 `finalize` 同持 `artifact_lock` 互斥（不产孤儿目录）。崩溃留下的 `generating` 孤儿由 `reconcile_orphaned_generating`（library-wall 加载时，注册表不含 + 陈旧 grace）翻 failed 占位。**非流式 `create_artifact_generating` 完整保留**作 agent 工具面 + image / 无 brief / 无 tokio runtime 兜底；`side_query` / recap / judge 等非流式路径字节不变。iframe 恒 `sandbox="allow-scripts"`（opaque origin，postMessage-only）、接收脚本零网络。

### 5.7 音频与交互组件形态（第 10、11 种）

在 9 个纯静态 HTML 形态之外，两种媒体/交互形态——都仍是「自包含 HTML + iframe 直载、浏览器零编译」。

- **`audio`（第 10 形态，媒体产物）**：prompt → 音频合成 → mp3 base64 **data-uri 内嵌 `<audio controls>` 播放器**（纯静态、零运行时、零网络，比 motion 还轻）。走统一[媒体生成](media-generation.md)栈（`media_gen::execute_audio`：服务商→多模型→speech/music/sfx 三条默认链 + failover **只在能服务该 kind 的候选间轮换**）；kind 可显式指定（生成对话框分段选择 / design 工具 `audio_kind`），缺省回退 `design/audio.rs::infer_audio_kind` prompt 前缀推断；voice 三层覆盖（调用级 `audio_voice` > 模型默认 > provider 默认）。`editable=false`（同 Image，无 oid）。配置归媒体生成三件套（`media_generation` category，服务商在「模型服务商 → 生成模型」、工具参数在「工具 → 媒体生成」）。
- **`component`（第 11 形态，交互式 React）——后端编译，浏览器零编译**：达到 Claude Artifacts 级真交互（state / 事件 / hooks / mini-app）而**不重蹈 atelier 白屏**——关键是编译搬到后端：
  - **`design/compile.rs`（oxc，纯 Rust、进程内、零外部二进制、零网络）**：LLM 产出的 JSX/TSX 源（classic runtime、全局 `React`、无 import/export）→ `Parser` → `SemanticBuilder::into_scoping()` → `Transformer`（`JsxRuntime::Classic` → `React.createElement`，默认 env = 只 JSX 转换 + TS 剥离、不降级现代语法）→ `Codegen` → 浏览器可执行 JS。
  - **`renderer::build_component_html`**：内联 **vendored React 18 production UMD**（`design/assets/{react,react-dom}.production.min.js`，`include_str!`、锁 React 18 因 19 删了 UMD、零网络）+ 编译产物 + bootstrap（`ReactDOM.createRoot(...).render(React.createElement(App))`）→ 静态 `index.html`。iframe 载已编译静态产物、`sandbox="allow-scripts"` opaque origin、零网络。
  - **失败必降级不白屏**：编译 `Err` → `build_component_error_html`（静态错误页，产物仍可开、可重生），**绝不 bail 阻断创建、绝不后端 panic**；`design::compile` 对畸形/截断源返 `Err`（单测锁「不 panic」）。
  - **生成**：`generate::generate_component_source`（side_query 产 JSX，早筛必含 `App`）；空白 Component 用 `placeholder_component_source`（合法 JSX 占位，非 HTML——HTML 当 JSX 会编译失败）。
  - **能力边界（刻意）**：Component 编译产物 ≠ 源码，故**不支持 oid 字节级可视化微调**（微调仍只归 9 静态 kind）；不走流式（回落阻塞 `create_artifact_generating`，编译一次 + 单次落盘）。

---

## 6. 设计系统层（品牌契约 + Token 编译）

### 6.1 `DESIGN.md` 规范：9 段 canonical schema + Token 表

品牌契约是**唯一真相源**的单文件 Markdown（`DESIGN.md`，规范实现见 `design/design_md.rs`）。9 段 canonical schema（`design_md::SECTIONS`，双语标题，导出按此序）：

1. **主题与品牌**（Brand）— 一句话定位 + 关键词
2. **色彩与角色**（Palette）— primary / secondary / accent / neutral / 语义色（success/warn/danger）+ 明暗
3. **字体排印**（Typography）— 字族 / 字号阶 / 字重 / 行高
4. **间距与网格**（Spacing）— 间距阶 / 栅格 / 圆角 / 阴影
5. **布局与响应式**（Layout）— 布局原则 / 断点行为
6. **组件样式**（Components）— 按钮 / 卡片 / 输入 / 导航 的形态约定
7. **动效**（Motion）— 过渡时长 / 缓动 / 60fps transform-opacity 约束
8. **语气与文案**（Voice）— 措辞 / 语气 / 词汇表
9. **禁忌与反模式**（Anti-patterns）— 明确不要做什么

文档末尾附 **Token 表**（`## Tokens` markdown 表，`--ds-*` CSS 变量）——机器可解析、可无损回灌，使每份 `DESIGN.md` 都是完整、可移植、可再导入的单文件。

### 6.2 Token 编译

`system::compile_tokens(system_md) -> tokens.json`：从 DESIGN.md 结构化区块解析出 CSS 自定义属性（`--ds-color-primary`、`--ds-space-4`、`--ds-font-sans`、`--ds-radius-md` …）。渲染时 `renderer` 把 `tokens.json` 展开为 `:root { … }` 注入产物。产物 CSS 引用变量而非硬编码 → **套用/切换设计系统即换皮，一致性由 token 锁定保证**。token 另可导出为 CSS / SCSS / TS / Swift / Android XML / DTCG 六种开发者格式（见 [§6.7](#67-多平台-token-导出designtokenexport)）。

### 6.3 内置设计系统（原创原型语言 + 品牌风格参考）

两类随 App 发行，都是完整 DESIGN.md + token，用户可 fork / 反向提取新建：

- **6 套原创原型语言**（`system.rs::builtins`）：极简现代、编辑杂志、科技暗色、温暖亲和、专业金融、大胆活力，覆盖常见气质光谱。
- **一批品牌风格参考**（`brands.rs` 的 `BrandSeed` 种子 → `system::expand` 展开为完整 25 token 契约）：覆盖开发者工具 / AI / SaaS / 设计框架 / 社交 / 媒体电商 / 大厂等主流品牌。每个种子只声明签名色 / 字体 / 圆角 / 字号密度 / 气质，`expand` 按背景明暗自适应补齐语义色 / 中性色 / 阴影，保证 token 契约齐全一致。**均为对各品牌公开视觉语言的独立再诠释，仅供设计参考**——`build_system_md` 对 `brand_ref=Some(..)` 的系统在摘要下自动附一行免责声明（非官方、无隶属 / 赞助 / 授权、商标归各自所有者），原创系统不附。

**分组与选择（`category` 字段）**：`BrandSeed` 按分节经 `brands.rs::cat(..)` 统一打上类目（开发者工具 / AI 产品 / SaaS / 设计框架 / 社交 / 媒体电商 / 大厂），原创系统类目为「原创原型」；`category` 落 `design_systems` 表（旧库启动期幂等 `ALTER TABLE` 补列 + `backfill_system_category` 仅填 NULL）随 `list_systems` 返回。GUI 侧 `DesignSystemPicker`（Dialog + 搜索框，规避菜单内输入焦点冲突）按 `category` 分组、按 name/summary/category 即时过滤；DesignView 头部与设置页「默认设计系统」共用。用户自建 / 提取系统 `category=None`，归「我的设计系统」组。

### 6.4 反向提取（D2 护城河）

`design(action="extract_system", from, ref)`：

- `from=image`（截图 / 设计稿）→ 视觉模型**直接看图**（`automation::run_vision`，全 4 种 Provider 格式 + 链级 failover / 非视觉候选自动跳过）→ 生成 `DESIGN.md` + `tokens.json`。模型 = GUI 选择器传入的 `model_override`（单模型不降级）或缺省默认链；**不再读 `function_models.vision`**（旧 `design/vision.rs` 自包含直发 HTTP 路径已随之退役删除——它当初存在只因彼时无 `run_vision`）
- `from=url` → `security::ssrf::check_url` 后抓取页面 + 首屏截图 → 提取
- `from=codebase`（本地代码工程）→ 读工程的 CSS / tailwind config / design token 文件 / 现有 `DESIGN.md` → 归纳 `DESIGN.md`

**owner 写入为主**：反向提取默认落 managed 设计系统目录（用户可见可编辑），**后台自主维护绝不写外部工程**（对齐知识空间外部只读红线）。

**前端入口（owner 平面）**：提取对话框对 `from=codebase`/`image` 提供**文件选择器**（桌面 `tx.pickLocalDirectory()` 选目录 / `tx.pickLocalImage()` 选图片，回填绝对路径），免手打路径——`PickedImage.path` 由 Tauri 原生 picker 回真实路径（HTTP 留空，故选择器仅 `supportsLocalFileOps()` 桌面显示、网页仍手填服务器路径）。`from=image` tab 另有**视觉模型选择器**（通用 `ModelSelector` 的 `requireVision` 置灰非视觉模型；与首页共享 `DesignConfig.last_model` 记忆）。**提取成功自动应用**新系统（对齐 picker 选择行为）：有打开产物则就地 restyle、否则设为项目默认（`extract_system` 返回新 `DesignSystemMeta` 供前端取 id 应用）。

**Figma 导入（工程轴 B，owner 平面专属）**：`extract::from_figma(url, token)` 经 `check_url` 拉 Figma REST API——优先读已发布 color/text/effect styles（`/v1/files/{key}/styles` + `/nodes`，Figma 颜色 `{r,g,b,a}` 0..1 浮点 → `#rrggbb[aa]`），无则回退遍历文档采样 SOLID 填充色（有界防超大文件）——汇成 material 后交同一 `run_extract` LLM 蒸馏成 9 段系统 + tokens。**凭据安全红线**：Figma 个人访问令牌**只走 owner 平面**（Tauri `import_figma_system_cmd` / HTTP `POST /api/design/systems/figma`）、**按次传入、绝不落盘、绝不进模型面**（`design` 工具无 Figma action）——与「强制留 GUI 的凭据例外」一致。SSRF 复用出站统一策略，无自写 IP 校验。

### 6.5 DESIGN.md 规范互通（导入 / 导出）

`DESIGN.md` 既是内部落盘格式，也是**跨工具互通格式**：

- **导入**（`service::import_design_md` / 工具 `design(action="import_design_md", content)` / owner `POST /api/design/systems/import`）：解析任意 DESIGN.md——`design_md::extract_tokens` 从 `:root{}` / 表格 / 内联抽 `--ds-*` token（≥4 个即确定性直用，**零 LLM 成本**）；token 不足时用 LLM 从正文合成，但**始终保留原 DESIGN.md 正文**（不改写用户 prose）。source 记 `imported`。
- **导出**（`service::export_design_md` / 工具 `design(action="export_system")` / owner `GET /api/design/systems/{id}/design-md`）：`design_md::to_design_md` 输出正文 prose + 末尾 Token 表，**可无损再导入**。
- **`from=codebase`** 反向提取本就读工程内现有 `DESIGN.md`，与导入互补。

### 6.6 设计变量可视化编辑（`DesignTokenEditor`）

逐 token 可视化手调（**两家云端竞品都没有的护城河**：一家改系统靠对话 Remix、另一家靠手改源文件）。owner 平面 `get_design_system_cmd` 载入某系统的 `tokens`（`--ds-*`）→ 前端按前缀（color / space / font / radius…）分组、逐 token 编辑：颜色值给取色器 + hex、其余给文本框，可**可视化 ↔ 源码**（`--key: value` 逐行）切换。保存走 `save_design_system_cmd`：`user`/`extracted` 就地更新；**内置只读系统 → fork 为「我的」新副本**（不传 id、`source=user`），并自动设为项目默认。落盘 chokepoint `system::save_system` 会用当前 tokens **重建 DESIGN.md 末尾 Token 表**（`design_md::replace_tokens_table`，剥旧表 + 附新表、保留正文 prose）——保证「编辑变量不改正文」时 `DESIGN.md`、`tokens.json`、导出/再导入三者永不漂移。

### 6.7 多平台 Token 导出（`DesignTokenExport`）

把设计系统的 `--ds-*` tokens 一键导出成开发者可直接落地的**六种格式**，供工程侧接入——这是把「设计系统」真正推到代码交付边界的工程轴能力。

- **纯函数生成**：全部在后端 `design/token_export.rs`（`export_all(tokens) -> Vec<TokenExport>`，**确定性、无网络、无副作用**）：
  - **CSS**：`:root { --ds-*: … }`
  - **SCSS**：`$ds-*: …;`
  - **TypeScript**：`export const tokens = { camelCase: "…" } as const` + 派生 `DesignTokens` 类型
  - **Swift (iOS)**：`enum DesignTokens`（颜色 → `UIColor(ds:)`，尺寸/数值 → `CGFloat` + 原值注释；含颜色时附一段 hex 解析扩展）
  - **Android XML**：`<color>`（CSS `#rrggbbaa` → Android `#aarrggbb` ARGB）/ `<dimen>`（px→dp 1:1、rem/em→dp×16）/ 其余 `<item>`
  - **DTCG**：Design Tokens Community Group 标准 JSON（按 `-` 分段嵌套 + `$value`/`$type`）
- **类型推断**：`classify(name, value)` 纯启发式（颜色 / 尺寸 / 时长 / 字体族 / 字重 / 数值 / 其它，值优先、名称提示兜底）决定各平台落地方式与 DTCG `$type`。
- **降级不产坏文件**：非 hex 颜色 / 无 Android 等价的视口单位 → 降级为注释或字符串资源，**绝不产出编译不过的文件**；空 token 也产出合法骨架。
- **两个平面**：owner 平面 `export_design_tokens_cmd`（Tauri）/ `GET /api/design/systems/{id}/tokens/export`（HTTP）供 GUI 导出对话框（Tabs × 6 + 复制 + 下载）；agent 平面 `design(action="export_tokens"[, format])` 让模型按需导出（缺省全部、`format` 取单个目标）。

### 6.8 绑定代码工程 + Token 同步（`DesignCodeBinding`，工程轴 D）

把设计系统**绑定**到一个代码工程目录，一键把多平台 token 文件**同步**写进去——让设计系统成为工程侧 token 的上游真相源，改 token → 重新同步 → 代码工程即时更新。这是工程轴的闭环终点。

- **数据**：`design.db` 的 `design_code_bindings` 表（`system_id` FK CASCADE / `target_dir` / `subfolder` / `formats` JSON / `last_synced_at`）；系统删除级联删绑定。
- **写盘安全边界（红线）**：所有写盘经 `service::resolve_binding_write_dir`——`target_dir` 必 canonicalize（须存在且是目录）、`subfolder` 拒绝绝对路径 / `..` 段、拼接后再 canonicalize 校验仍 `starts_with(root)`（防 symlink 逃逸）；写用 `platform::write_atomic`（**禁 `fs::write`**）。token 文件名固定（无用户输入），另写 `DESIGN_TOKENS.md` 溯源清单（specific 名避免撞项目 README）。
- **凭据 / 平面（红线）**：**owner 平面专属**——`bind`/`sync` 是外部工程写操作，**HTTP 侧受 `filesystem.allowRemoteWrites` 门（默认关）**，桌面 Tauri 不受限；**`design` agent 工具无绑定 action**（模型不能自主往用户代码工程写文件）。`unbind` 只删绑定记录、**不删已写文件**（那是工程侧资产）。
- **入口**：Tauri `bind/sync/list/unbind_design_code_*_cmd` / HTTP `POST /api/design/bindings`·`POST …/{id}/sync`·`GET …/bindings`·`DELETE …/bindings/{id}`；前端 `DesignCodeBinding` 对话框（复用 `useDirectoryPicker` 选目录 + 格式多选 + 绑定即同步 + 逐绑定「同步 / 解绑」）。

### 6.9 关联代码仓库（项目级双源绑定，工程轴 E）+ 实现到代码

把**设计项目**绑到一个真实代码仓库，让设计空间与代码工程形成一等关联——这是与 §6.8（设计系统级、token 写出方向）正交的另一层：§6.9 管「读授权 + 会话锚定 + 实现落地」，§6.8 管「token 同步写出」。

**双源绑定（互斥二选一）**：
- **本机目录源** `design_projects.code_dir`：canonical 绝对路径（`set_project_code_binding` 绑定期 canonicalize + 存在性校验）。
- **HA 项目源** `design_projects.ha_project_id`：目录从该 Hope Agent 项目的 working_dir **实时派生**（显式 `working_dir` > lazy 默认 workspace，与会话工作目录合并同款语义）——用户改 HA 项目工作目录自动跟随。
- **解析单一入口 `service::resolve_code_dir(project)`**（code_dir > ha_project_id > None）；任一源失效（目录被删 / HA 项目被删 / **HA 项目显式 working_dir 指向不存在路径**）一律 **fail-safe 返 None、按未绑定处理**（HA 源绝不静默回落 lazy workspace 掩盖 stale），不自动清列，GUI 经 `get_project_code_binding` 的 `stale` 标记标红（对齐 cron delivery target stale 先例）。
- **互斥单一写入口（红线）**：`code_dir` / `ha_project_id` 两列的**唯一**写入口是 `service::set_project_code_binding`（双源同传 bail、canonicalize、HA 存在校验、db 层 verbatim 覆写非 COALESCE）；`create_project` / `update_project` **不碰这两列**（否则 owner update/create API 可绕过互斥同时写两源、破坏不变量）。

**绑定生效面（四处）**：① 反向提取对话框 codebase 通道预填生效目录；② **agent 提取读根扩张**——`scoped_local_path` 的允许根从「会话工作目录 ∪ 附件」扩为「∪ 绑定仓库」（经 `session_bound_code_dir`：会话 → **设计线程锚（确定）**> `session_id` 弱引用兜底**仅在恰一个项目时采纳**，多个则 None + warn 拒绝按 `updated_at` 顺序隐式翻转授权根 → `resolve_code_dir`，只读不建项目），fail-closed 语义不变；③ **设计线程会话 working_dir 实时派生**——设计线程（`kind=Design`）无 project_id，其工作目录由 `session::effective_working_dir_for_meta` 经 `session_bound_code_dir` **实时派生**（不再事件时拷贝到 sessions.working_dir）：HA 项目 working_dir 后续变更 / 绑定切换 / 解绑**立即反映、绝不 stale**，mint 路径零阻塞 IO；设计对话里的 agent 由此能用 `read`/`exec` 真读仓库（受既有权限引擎管辖）；④ `DesignCodeBinding` token 同步目标目录预填。design 库句柄经 `get_design_db()` 进程级缓存（热路径不每调重放 DDL）。

**红线（与 §6.8 同族）**：绑定 = **用户显式授权读取该目录**——**owner 平面专属**（Tauri `get/set_design_project_code_binding_cmd` / HTTP `GET/PUT /api/design/projects/{id}/code-binding`），**`design` agent 工具无绑定 action**（模型不能自授权扩读根）。绑定本身只写 design.db + 会话行、不落外部文件，故 HTTP 面不过 `filesystem.allowRemoteWrites` 写盘门（与 extract 的 owner 读同级信任）。

**实现到代码（`implement_to_code`，设计稿 → 组件级落地）**：产物导出菜单「实现到代码…」把设计稿交给**正常 chat 会话**在绑定仓库里实现——对标 handoff-bundle-给-coding-agent 的语义，但落在自家 agent（结构优势：无需外部 CLI，权限引擎 / DiffPanel / Plan Mode / worktree 全复用）。
- **后端只做三件事**（`service::implement_to_code`）：`build_implement_pack`（复用 handoff 素材函数 `read_source`/`referenced_tokens`/`export_design_md`，纯只读：产物源码分段截断上限 + 引用 token 表 + 未解决画布批注 + DESIGN.md 摘要 + 实现指令模板）+ 建会话（agent 取项目 `agent_id` 回退默认解析链；**working_dir 写入失败整体 fail**，绝不让实现会话落错 cwd）+ 返回 `{session_id, prompt, code_dir}`。**不在后端发起 turn**。
- **前端接线**：DesignView 未绑定先引导关联；已绑定 → `App.onImplementToCode` 设 `pendingSessionId` + `pendingAutoSend{sessionId, message, nonce}` 切主对话，ChatScreen 在目标会话就绪后经 `stream.handleSend` 把 pack 作首条消息发出（nonce + ref 双防重放）——流式 / 审批 / DiffPanel 全走既有路径。
- **红线**：写代码的每一笔都发生在 chat 会话的权限门之内（write/edit 逐笔审批）；`design` 工具自身仍无任何仓库写动作；pack 是用户显式动作产物、不含自动授权语义；incognito 不可为实现目标（design 工具入口已 fail-closed）。

**code→design 回灌（stale 检测 + 引导更新，`code_sync` / `code_watcher`）**：`implement_to_code` 落地后，代码侧的后续改动应让设计空间「知道」——否则 coding 与 design 交替时产物漂移。数据链路 **回执 → 收割 → 比对 → 三动作**：

- **回执**（`design_implement_receipts`）+ **链接**（`design_code_links`，两级 `ON DELETE CASCADE`）：`implement_to_code` 尾部建回执锚定「产物 → 承接会话 → 落地目录 + git 基线 revision + 收割游标」。**收割**（harvest，增量幂等）从承接会话的 `write`/`edit`/`apply_patch` 工具元数据（`load_session_messages_after` 游标批读，镜像 `session/artifacts.rs` 的 `file_change`/`file_changes` 解析，越界路径 `starts_with(code_dir)` 丢弃）提取「产物落地文件」→ 逐文件 BLAKE3 + gzip 快照存为基线。**语义自洽**：游标增量让实现会话自己的后续改动被吸收为基线，**只有会话之外的外部改动被判为漂移**。二进制探测与文件浏览器同口径（`filesystem::looks_binary_bytes`，NUL 或非 UTF-8）——GBK/Latin-1 等非 UTF-8 源码不存快照、diff 出 before/after 皆空占位，不再 `from_utf8_lossy` 出 U+FFFD 乱码。
- **比对**（`check_code_drift`，打开项目/产物、手动按钮、watcher 统一入口）：逐 link 先比 size 短路（`fs::metadata` 不同即 modified、免读免哈希），相同才**流式 BLAKE3**（有界内存，不整读大文件——防已收割路径后被写成 GB 级产物 OOM）；缺失=deleted → **原子写**产物 `metadata.codeDrift` 单键（`set_artifact_code_drift` 走 SQL `json_set`/`json_remove`，只动本键、**不占 status 列、不 bump `updated_at`**，消除与前台整列 metadata 写互相丢键的竞态），翻转才 emit `design:code_drift`。**读回红线：绝不跟随 symlink**——`resolve_linked_path` 判文件本体非 symlink + `canonicalize` 复验仍在 root 内（挡已登记路径被 git checkout / npm postinstall 换成指向仓库外 `~/.ssh/id_rsa` 等 symlink 的越界读，否则内容会经 diff / quote / 基线快照外泄）；`code_dir` 失效跳过不假 stale（绑定级失效由 `CodeBindingInfo.stale` 另标红，正交）。项目卡 `code_drift_count` 聚合走 `json_extract(metadata,'$.codeDrift')`。
- **实时监听**（`code_watcher`，照抄 `knowledge/watcher.rs`）：只 watch 已收割落地文件的**父目录集（`NonRecursive`）**、按关联绝对路径集精确过滤（规避 node_modules 洪泛 / inotify 配额），800ms debounce → `check_drift_for_dir`（**先收割后比对**——与 `check_code_drift` 同一收割入口，游标增量把实现会话自己刚落盘的改动吸收为基线，避免会话进行中 watcher 把自身改动误报为外部漂移）；索引变化后 `refresh_all` 全量重建；`app_init` 启动挂载（Primary）。父目录被删 watch 静默失效，由打开时确定性检查兜底。收割游标停在在途流式行之前（工具行两阶段落库，避免回填后漏建 link）。
- **三动作**：**查看代码变更**（`drift_changes` 组 `FileChangeMetadata` 兼容形状复用 `tools::diff_util` + 前端 `DesignCodeDriftModal` 内嵌既有 `DiffPanel`）/ **带到设计对话**（`quote` pack 塞 composer、预填指令、不自动发，对齐批注先例）/ **标为已同步**（`mark_synced` 重置基线 + 清标）。
- **红线**：只读已授权绑定目录 + 写 design.db，不落外部文件（HTTP 面不过 `filesystem.allowRemoteWrites`，同 `set_code_binding` 理由）；`build_implement_pack` 指令要求改动走 `write`/`edit`/`apply_patch` 工具落盘（exec/heredoc 绕过元数据 = 收割漏 link 的已知边界）。**解绑 / 换绑清理**：`set_project_code_binding` 绑定源真变时 `delete_receipts_for_project`（links 级联清）——否则 watcher 与 `check_code_drift` 仍按旧 links 读已撤销授权的目录。回执创建（`create_receipt_for_implement`）对 `implement_to_code` 是 **best-effort**（失败 warn 不 propagate）——会话已建好且回执写独立 design.db，硬失败会毁 implement 主流程留孤儿会话。

---

## 7. 可视化直接微调（选中→反查→回写）

这是 D1，也是旧版做不好、本版从架构层解决的能力。**根因分析**：旧版产物是编译后的 React 组件，用户在 DOM 上的改动要反查回 JSX 源，中间隔着 React 渲染与 Tailwind 编译，映射有损、经常改不动或改坏。**本版产物是纯 HTML，渲染 DOM 与源码结构一一对应，回写是确定性的。**

### 7.1 oid 映射（渲染期建立）

`renderer` 在编译产物时，遍历 body HTML 的每个元素，注入 `data-ds-oid="{n}"`（源码文档顺序编号），同时产出 `oidmap.json`：`{ "12": { start: 3480, end: 3560, tag: "button", ... } }`（源码字节范围）。oidmap 随版本落盘。

**渲染版本自愈（bridge 烧死在产物里的解法）**：inspector bridge 与 oid 注入是**编辑工具层**，`build_artifact_html` 时**烧进 `index.html`**（预览 iframe 加载静态文件）。工具层升级后，**老产物的 `index.html` 仍是旧 bridge**（不重编辑就用不上新工具）。故可编辑态 `index.html` 的 `<html>` 带 `data-ds-r="{RENDER_VERSION}"`（`renderer::RENDER_VERSION`，工具层变更时 +1）；打开产物时 `service::ensure_artifact_render_fresh` 检测版本落后即**用当前 renderer 从磁盘源静默重渲染 `index.html` + `oidmap.json`（内容不变、不新增版本、不动 source）**，前端据返回值 bump `previewKey` 重载。仅 `status=ready` 的可编辑 kind 执行、已最新即 no-op（owner 平面 `ensure_design_artifact_fresh_cmd` / `POST …/ensure-fresh`）。

### 7.2 交互三通道

1. **对话改写**（自然语言）：让 AI 改，产出新版本，可要多个变体并排。
2. **就地直接编辑**（选中产物进入编辑态）：
   - 点选元素 → bridge postMessage `{oid, tag, computedStyle, textContent, rect}` → `DesignInspector` 显示 **8 分区控件**（文本 / 颜色 / 排版 / 间距圆角 / **布局**（display + flex 时 align/justify/gap）/ **尺寸**（宽高/最大宽/最小高，自由 CSS 值）/ **描边**（边框宽/样式/色）/ **效果**（不透明度滑杆 + 阴影预设））。bridge `CSS_PROPS` 报全量 computedStyle，控件即时 `onLiveStyle` 预览、交互结束 `onCommitStyle` 确定性回写。
   - 改控件 → bridge 即时把 inline style / 文本应用到 live DOM（**零延迟乐观预览**）→ 交互结束 commit：owner 端 `patch_element(artifact_id, oid, patch)` 按 oidmap 定位源码字节范围，确定性回写（生成新版本）。
   - 文本双击 → contenteditable → commit 写回文本节点源码范围。
3. **批注钉**（`design_comments` 表 + `design::selfcheck` 无关的独立 CRUD）：批注模式点选元素落**元素锚定钉**（`oid` + 元素内相对坐标 `rel_x/rel_y` + 元素摘要 snippet）；bridge 在 iframe 内渲染钉（坐标随锚元素、zoom 无关），设计变化后按 `oid`→snippet 前缀**软着陆重锚**（漂移不丢，脱锚回退角落堆叠），可**拖钉手动重锚**（确定性回写 rel + oid，owner 端 finite/clamp 校验）。**回灌对话**（`design_comment_refine_cmd`）= design-space 原生：AI 按批注结构化上下文（反馈 + 元素 + snippet + 当前设计）精修产物、**就地落新版本**（`design:reload` 刷新，无需切走），复用生成管线（image/audio/component 形态不支持）。批注可标记已解决 / 编辑 / 删除。owner 平面 CRUD（Tauri / HTTP `/api/design/artifacts/{id}/comments…`），坐标为沙箱回传的不可信数值、全经 owner 端钳制。**未解决数量 badge**：`ArtifactView.open_comment_count`（`db.count_open_comments` 走 `(artifact_id,resolved,id)` 索引轻量 COUNT）随 `openArtifact`/`refreshView` 免费带出，工具栏批注按钮据此渲染角标——**不进批注模式亦可感知**待处理批注；前端 `loadComments` 单点回填该计数（增删 / 解决 / 精修即时刷新，不触发 iframe 重载）。

### 7.3 回写红线（对齐知识空间 / atelier 已验证的安全点）

- **沙箱消息不可信（净化以主机侧为准，B0-7）**：iframe → 磁盘写是首个不可信写通道。**权威净化在后端 `patch.rs`**（沙箱消息可伪造，前端校验只作 UX、绝不当边界）：① CSS 值走**函数白名单** `SAFE_CSS_FUNCTIONS`——calc/var/color/gradient/transform/filter 等合法函数放行，`url()` / `image-set()` / `expression()` 等可加载远程资源或执行的向量**整值拒绝、返回空即跳过该声明**（守自包含零网络红线；黑名单永远列不全，故用白名单）+ 结构性字符 `< > " ; { }` 过滤 + 属性名限 `[a-z0-9-]`；② oid 经 oidmap `find_entry` 主机侧校验（不在图 → `OidNotFound` 拒）；③ 前端数值输入 NaN/空**回填原值不 commit**（不再 `parseFloat||0` 静默写 0 抹掉尺寸；负值仍合法不钳）。
- **确定性命中**：`patch_element` 按 oidmap 字节范围唯一命中；命中 0 处或源已变（`expected` hash 不符）→ 拒绝（stale-write 守卫），前端提示"源已更新，请重新选中"。
- **撤销/重做 + 版本**：每次 commit 建新版本，可 restore。
- **单一稳定 iframe**：编辑面是一个固定 iframe，无画布 transform，无 pointer capture 自研逻辑 → 无卡顿、无拖拽泄漏。

### 7.4 画框批注（B4-1，自由绘制标注 → 截图带到对话）

「点选元素微调」之外的**自由区域标注**：用户在预览上框选（box）或画笔（pen）圈出要改的区域，连同说明作为**带红框标记的设计截图 + 范围约束指令**带到对话让 AI 就地精修。三态互斥于 editMode / commentMode（`DesignView` 工具栏，仅 `isEditableKind` 可用）。

- **父层叠层，零沙箱依赖**：绘制层是浮在预览 iframe **之上的父层 `<canvas>`**（`DesignDrawOverlay`，仅 drawMode 期条件挂载 → 卸载即复位）。iframe 跨源（`asset://` / HTTP）+ sandboxed，父层本就无法读进去，故绘制只发生在父层。**笔画/框一律归一化存 `0..1`**（相对 canvas 矩形），与分辨率无关——这是可靠合成的关键不变量。
- **底图捕获走离屏栅格化（不截活 iframe）**：跨源 iframe 无法直接光栅化，复用 export 同款**自包含 HTML → 同源隐藏 iframe → html2canvas**（`designExport.rasterizeArtifactFull`，桌面 / server 通用、无 Chrome 依赖）。**按 `clientWidth` 视口渲染**以复现屏上布局；倍率钳到 `MAX_CANVAS_PX` 防超 WKWebView 上限静默出空白。合成把红框/红线画到底图再裁剪到 marks 包围盒。
- **坐标映射单一真相**：叠层 canvas 尺寸 = iframe **可视 footprint**（纯宽高、无 transform；device/zoom 模式若用 `inset-0` 会因 border-box 边框收窄 content box 而漂移，故必须贴 footprint）。映射 `ax = scrollX + nx*clientWidth`、`px = ax*renderScale`；`scrollX/Y`·`clientW/H` 经 **`ds_viewport` 桥回传**（父层跨源不可直接读滚动/视口），滚轮经 **`ds_scroll_by`** 转发给 iframe。两条桥消息 benign（纯读滚动数 / 纯滚动），与 `ds_activate` 编辑激活正交、不受 active 门控。
- **送达对话复用主对话附件面**：合成图经 `DesignChatPanel.addImageAttachment` 作 vision 图附件、区域描述 + note 作 scope quote，用户审后手动发（draft 语义，不自动发）。
- **高可用降级红线**：捕获失败（桥超时 / 栅格化异常 / 空产物）**永不阻塞**——静默降级为「区域 + 文字」纯文字标注；**deck / motion 等多帧多态产物跳过底图**（离屏 fresh render 只渲默认帧、与用户所看不符会误导，宁缺勿错）；内容刷新（`previewKey` 变、iframe 重挂）时叠层随之重挂弃旧 marks，不落到重排后的错位处。

---

## 8. Agent 工具面（`design` 工具）

单一 `design` 工具（`internal: true`，按 action 路由；生成类 `async_capable`），供模型自主创建/迭代设计。

| Action | 语义 | 平面 |
| --- | --- | --- |
| `list_systems` / `get_system` | 浏览 / 读取设计系统契约（供生成时 grounding） | agent |
| `list_recipes` / `get_recipe` | 浏览 / 读取产物模板指令 | agent |
| `create_system` / `extract_system` | 新建 / 反向提取设计系统 | agent（提取默认落 managed，不写外部工程） |
| `list_projects` / `list_artifacts` | 浏览项目与产物 | agent |
| `get_artifact` | 读产物 + **oid 标注的当前源码**（`source.body` 每元素带 `data-ds-oid`、`source.css/js/bodyHash`）——编辑前先读，让 agent「看得到再改」 | agent |
| `create_artifact` | 生成产物（kind + system + html/css/js），渲染 + 预览 | agent |
| `update_artifact` | **整段** body/css/js 重写（大改才用；小改勿用，防抹空） | agent |
| `edit_element` | **按 oid 就地精改一个元素**（style/text/attrs/remove），保留其它一切——小改（改色/改文案/改间距/换属性/删元素）的正道；复用确定性 `patch_element`（owner 可视化微调同一后端）+ stale-write 守卫 | agent |
| `delete_artifact` / `versions` / `restore` | 删除 / 版本列表 / 恢复 | agent |
| `snapshot` | 渲染 PNG（缩略图 + 多模态自反馈回路） | agent |
| `critique` | 5 维质量门评审 | agent |
| `propose_directions` | 无设计系统时给 N 个方向选项 | agent |
| `export` | 导出 HTML/PDF/PPTX/PNG | owner + agent |
| `save_to_knowledge` | 沉淀产物为 KB 笔记（D4） | agent |

**关键不变量：**

- **kind 不可变**：`update` 沿用 `create` 时的 kind，换类型只能删建。
- **小改就地精改、勿整段重造（红线，源自实测翻车）**：改一处（色/文案/间距/属性/删元素）走 `edit_element(oid)`——`get_artifact` 先回 oid 标注源让 agent 定位，`patch_element` 确定性回写、保留其它一切；`update_artifact` 只留给大改。**曾实测：agent 看不到当前内容→`web_fetch` 抓产物失败→凭记忆整段重造→把整页抹空**；工具描述明令「小改用 edit_element、绝不为小改重造、绝不 web_fetch/浏览产物读它」。仅 `supports_oid_edit` 的 kind（非 image/audio/component）可 `edit_element`。
- **snapshot 自反馈**：`snapshot` 返回值走 `IMAGE_BASE64_PREFIX`（与 browser/canvas 共用），执行层物化为多模态 image 输入 → 模型能"看到"自己的设计并迭代。
- **owner 覆盖不进 agent schema**：涉及外部工程写入 / 权限的动作是 owner 专属（模型 schema 不暴露），防注入提权。
- **批注带到对话即带 oid**：锚定批注「带到对话」的 quote 附 `oid` + 硬范围提示（仿 open-design `<attached-preview-comments>`），让 agent 直接 `edit_element(oid)` 一把改到位。

---

## 9. 前端视图与工作台

蓝本参考 [`KnowledgeView.tsx`](../../src/components/knowledge/KnowledgeView.tsx) 的 Header + 多栏可拖拽可折叠骨架，但**更简单、更稳**（无画布）。

### 9.1 三层结构

- **首页 `DesignHome`**：顶部大输入框（prompt-first，一句话起步直达生成）+ 产物类型卡（web/mobile/deck/dashboard/poster/document/email/image）+ 最近项目**缩略图墙**（纯 CSS grid，无画布）。
- **工作室（`DesignView` 项目态）**——左对话 / 右预览（chat-to-edit 布局，「对话改写」是头号迭代路径）：
  - 左：**AI 对话面板 `DesignChatPanel`**（可拖宽 320–640px · 可折叠，宽度 localStorage 持久化）——复用主对话 `useChatStream` + `ChatInput` + `MessageList`，会话是每项目独立的设计对话线程（`useDesignChat`，见 [§9.4](#94-每项目-ai-对话线程)）。这是模型迭代产物的主入口：自然语言让 AI 改，产物经 `design` 工具就地落新版本、`design:reload` / `design:generate_delta` 回刷预览。
  - 中：**单产物聚焦预览**——一个稳定 iframe，顶部**「已打开产物」标签条**（编辑器式工作区，只列已打开的产物、非全部；含对话折叠钮 + 末尾「从产物库打开」下拉入口）+ 缩放下拉（适应 / 百分比），纯 CSS `transform: scale()` 缩放，**无平移画布**。
  - 右：**检视抽屉 `DesignInspector`**（选中元素时滑出）：属性 / 代码 / 设计系统 / 批注。
- **多产物概览走产物库墙**（`LayoutGrid` 切换出 `DesignFilesPanel`），标签条只承载「已打开的工作集」。**不做无限画布**。

### 9.2 状态与交互

纯 `useState`（对齐 KnowledgeView，无 Redux）；栏宽/折叠 localStorage 持久化；`getTransport().call("*_cmd")` 与后端交互；`tx.listen("design:*")` 增量刷新。会话切换时 transient UI 状态（全屏/缩放）强制清零。

**页面组织（一个项目 = 一个应用，每个产物 = 一个页面）**：顶部是**「已打开产物」标签条**（编辑器式工作区，只列已打开的产物、非全部），配一个 studio 内「所有页面」缩略图总览（`LayoutGrid` 切换出 `DesignFilesPanel`，复用 `ArtifactThumb` 懒挂载沙箱缩略图；`component` kind 客户端水合、`sandbox=""` 禁脚本 → 用类型图标占位）。**标签开关 ≠ 删除（红线）**：标签上高频动作是**关闭**（`✕` / 右键「关闭」/「关闭其他」——仅从标签条移除，`.md` 文件与 `design.db` 行不动），**删除（永久）降级到右键菜单 + 产物库墙**（`delete_design_artifact_cmd` 硬删整个产物目录、不可逆）；关闭的产物从**产物库墙 / 标签条末尾「从产物库打开」下拉**随时重开（每个激活入口统一经 `openArtifact → ensureTabOpen` 挂成标签）。已打开集合每项目落 `localStorage`（`ha.design.openTabs.<projectId>`，纯视图状态、无后端、`design.db` 仍是全部产物真相源）：进项目**精确恢复上次留下的标签**，无记录（首次进）才默认打开最近一个，关到空则尊重空态、不擅自打开；产物删除 / 批量删后经 prune 修剪孤儿标签。页面**双击标签就地改名**（轻量 `rename_design_artifact_cmd`：仅改 `title`、不重渲染不新增版本）、**复制页面**（`duplicate_design_artifact_cmd`：同项目深拷贝 + 版本行，持 `artifact_lock` 且拒绝拷 `generating`）、**库墙内拖动排序**（`design_artifacts.position` 列，`reorder_design_artifacts_cmd` 按序写 position；`list_artifacts` 改 `ORDER BY position ASC`；`create_artifact` 位序自增追加）。项目名在工作室标题**点击就地改**（复用 `update_design_project_cmd` 的 title，乐观同步 `activeProject`）。

**文件夹分组（复刻参考产品的 path-based 模型）**：产物墙升级为 `DesignFilesPanel`（面包屑 + Folders 区 + 类型分区 + 缩略图行）。**文件夹 = 斜杠路径、非独立实体**——`design_artifacts.folder`（`TEXT NOT NULL DEFAULT ''`，空 = 根）存归属，无 folder 表 id / parent 树；空文件夹另落 `design_folders(project_id, path)` 注册表（否则无产物的空文件夹不可持久）。`list_folder_paths` 把「产物 folder ∪ 持久化空文件夹」连同全部祖先段并进 `BTreeSet`（去重 + 补全中间层，面包屑逐级可达）。移动 = 改 `folder`；文件夹改名/移动 = `rename_folder_prefix` 前缀替换（exact-match + `folder LIKE from/%` 子路径 `substr` 拼接）；删文件夹 = `detach_artifacts_from_folder` 把子树产物退回根（**不删产物**，本地选定的安全语义）+ 删注册记录。service `sanitize_folder` 去空段 / 拒 `.`/`..`、`rename_folder` 拒绝移进自身子孙（防前缀二次处理错乱）。**SQL 红线**：`LIKE` 模式经 `escape_like`（转义 `\`/`%`/`_`）+ `ESCAPE '\'` 子句，防文件夹名里的 `_`/`%` 误匹配兄弟；`substr` 起点用 `chars().count()`（SQLite `substr` 按**字符**计数，非字节）避免多字节名子路径截断。owner 双平面：Tauri `{list,create,rename,delete}_design_folder_cmd` + `move_design_artifact_to_folder_cmd`，HTTP 对齐（DELETE 走 `Query` 取 `path`），逻辑全在 `design::service`。前端拖拽到文件夹行 / 面包屑或 `⋯` 菜单移动，reorder 只在当前文件夹内重排后重映射回全量顺序；创建产物落根文件夹故建后收起面板直呈预览，reorder 乐观更新即时反映。

### 9.3 侧边栏入口

在 [`IconSidebar.tsx`](../../src/components/common/IconSidebar.tsx) 的「知识空间」入口**正下方**插入「设计空间」入口（lucide 图标，建议 `Palette` / `Shapes` / `PenTool`，`t("design.title")`，`view === "design"` 高亮，`onClick={onOpenDesign}`）。同步 `App.tsx` 的 `view` 联合、`lazy` import、渲染分支、`onOpenDesign` prop。

### 9.4 每项目 AI 对话线程

设计对话与知识空间侧边栏对话**同架构**：一个内嵌 chat 架在主对话栈上，scoped 到容器（知识空间→KB + 锚笔记；设计空间→设计项目）。前端 `useDesignChat`（镜像 `useKnowledgeChat`）只管会话生命周期 + model/agent 状态，流式/发送交给面板里的 `useChatStream`。

- **会话身份**：`SessionKind::Design`（`session/types.rs`）——持久化但从主侧栏 / `/sessions` / 全局 FTS 隐藏（`session/db.rs` 的隐藏谓词与 `knowledge` 同源，改为 `kind NOT IN ('knowledge','design')`）。**不是安全边界**。
- **锚定表 `design_chat_threads`**（sessions.db，`session/db.rs` 建表）：`session_id`（PK，FK sessions ON DELETE CASCADE）+ `project_id`（**纯列，无跨库 FK**——设计项目行在 design.db）+ `created_at`。方法在 `design/threads.rs`（走全局 `SESSION_DB`，JOIN sessions/messages 供历史选择器）。设计项目删除时 `service::delete_project` 先 `thread_session_ids` 收集并删这些隐藏会话（显式级联，非 ON DELETE）。
- **提升分支**：`chat` 命令新会话且 `tool_scope == "design"` 时，`mark_session_as_design_thread`（先建 thread 行再翻 `kind`，best-effort，镜像 KB 的 `mark_session_as_kb_thread`）锚到 `design_project_id`；前端仅在 auto-create send 携带（`useChatStream.draftDesignProjectId`）。Tauri + HTTP 双写；被 hook 阻断的首条消息丢弃僵尸会话（drop 分支含 `design`）。
- **工具面收窄**：`ToolScope::Design`（`tools/mod.rs`，`is_design_scope_tool` 白名单 = `design` + `web_search`/`web_fetch`/`image_generate`/`audio_generate` + `recall_memory`/`memory_get`/`knowledge_recall` + 框架基础）——**纯 schema/可见性收窄，非安全边界**；`design` 工具仍受 `app_config.design.enabled` 门控、incognito fail-closed。
- **项目解析**：`design` 工具经 `service::get_or_create_session_project` → **优先** `threads::project_for_session(session_id)` 命中锚定项目，未命中回落原「按 session 查/建草稿」逻辑（ACP 无 `SESSION_DB` 时静默回落）。
- **当前产物上下文**：面板 `getExtraAttachments` 每轮注入一条不可见 `<design_context>` quote（project_id + 打开的 artifact id/title/kind + 设计系统名），让「改这个 / 当前 / restyle 它」落到用户正看的产物；结构化、非 system 指令，模型仍走 `design` 工具实际操作。
- **owner 命令**：`design_chat_thread_get_cmd`（最近线程 SessionMeta，默认加载）/ `design_chat_threads_list_cmd`（历史分页 + FTS）——Tauri + HTTP `GET /api/design/projects/{projectId}/chat/{thread,threads}` + COMMAND_MAP，镜像 `kb_chat_thread*`。
- **批注两条出口**：批注卡「带到对话」经面板 `DesignChatPanelHandle.addQuote` 把反馈作 quote chip 塞进 composer（`DesignView` 持 `chatPanelRef`，展开被折叠的对话栏）——用户可补充后随 turn 发、AI 在完整上下文下迭代（批注→composer 直通）；「一键精修」仍走 `design_comment_refine_cmd` 单条快捷精修。
- **空态 starter + 新产物自动聚焦**：无消息时对话展示设计起步卡（点击填入 composer 不自动发）；chat-first 生成新产物时 `design:artifact_generating` 在无 active 产物时自动 `openArtifact` 生成壳，让流式直接在预览渲染。
- **发现问卷 + 视觉风格卡（`ask_user_question` 扩展）**：设计 Agent **需求不清才问**（不是进门必填的表单，也不是每次都问）——在对话里弹结构化发现问卷（single/multi/text/textarea），要定视觉方向时弹 `direction-cards` 风格卡（选项带调色板 + 字体样张 + 气质 + 参考）。走**扩展后的 `ask_user_question`**（非 fork），前端经 `useAskUserPending` 把问答接进设计对话、`DesignChatPanel` 传 `askUserVariant="design"` 让 `AskUserQuestionBlock` 富渲染风格卡（主对话/IM/历史降级为选项列表）。答案仍走 `selected[]`/`custom_input`，Yes/No 门与 IM 协议零改动；风格卡色值/字体在主 App 渲染故经 `sanitizeCssColor`/`sanitizeFontFamily`。契约详见 [`ask-user.md`](ask-user.md#input_kind-富输入类型--direction-cards-视觉风格卡设计空间)。**首页已删静态「补充简报」表单**——需求补全统一交给对话按需追问，不再在首屏叠一块两头不占的死表单。

### 9.5 设计系统套件视图（Kit，B1-1）

让抽象 token 表「看得见」——把一个设计系统渲染成可视套件页在沙箱 iframe 里预览。

- **后端生成自包含 HTML**：`design/kit.rs::build_kit_html(name, tokens)` 用系统 tokens 生成一张自包含套件页（色板 / 字体族 specimen / 字号阶 / 间距条 / 圆角+阴影 / 组件 showcase：button·input·card·badge），**全部引用 `var(--ds-*)`**——套件即系统真实视觉。token 注入复用 `renderer::tokens_root_css`（同一安全过滤：仅 `--ds-*`、值滤 `}{<;`）；名称/值经 `html_escape`。与产物同架构：**浏览器零编译、零网络**，`sandbox="allow-scripts"`。
- **owner 命令**：`get_design_system_kit_cmd(id)→String`（Tauri）/ `GET /api/design/systems/{id}/kit`（HTTP 返 JSON 字符串，两态 `call<string>` 通用）+ COMMAND_MAP。前端 `DesignKitModal` srcDoc 进 iframe；入口 = `DesignSystemPicker` 每行「预览套件」（`onPreviewKit`），浏览/换系统时可先看再选。
- **决策账本（2026-07-11 更新，原「表面切换」分歧已反转）**：dark / compact 现走 **单 seed 确定性派生**（`design/theme.rs` 的 `derive_dark` / `derive_compact`，HSL 保色相调亮度）——bg→近黑、fg→近白、中性色暗化、accent 类钳最低亮度保暗底可读；compact 缩放字号 / 间距 / 圆角。Kit 页 `body.dark` / `body.compact` 用派生的真 token（非旧「4 中性色 flip」）。**推翻原「不臆造暗色 token」分歧的理由**：确定性派生算法从单一 light token 集**算**出变体、无需手写维护第二套，与「不臆造」不矛盾。theme.rs 纯函数、零外部依赖、单测锁色相保持与尺寸缩放。**待办**：token 导出（`token_export`）与 TokenEditor 的双主题面。
- **B1-2 实时预览**：`TokenEditor` 双栏（左 token 编辑 / 右套件 iframe）；kit 页含 `<style id="ds-live">` + `postMessage` 监听，编辑器 token 草稿变化防抖 200ms 把当前 `:root` 覆盖 post 进 iframe **活重染**（结构 HTML 一次性取、零重取），值滤 `{}<;` 对齐后端注入安全。
- **B1-4 提取资产（logo / 配图）**：`from_url` 在 LLM 提取之外**确定性 harvest**——`parse_asset_candidates` 用 `scraper` 按优先链（logo：`apple-touch-icon > og:image > favicon > 带 "logo" 的 img`；imagery：`og:image > twitter:image > 前若干 img`，去 svg）取候选 URL、绝对化去重、截断尝试预算（logo 8 / image 14），`fetch_asset` 逐个 **经 `security::ssrf::check_url` 抓取**（复用红线不自写 IP 校验、size-gate `[256B,6MB]`、超上限弃）、content-hash 跨类去重、转 **data-uri**（自包含）。`ExtractedSystem.{logos,images}` → `system::write_assets`（`assets.json`）→ `DesignSystemFull.assets` → `build_kit_html` 渲染 Logo/配图段（**仅放行 `data:image/`**，滤掉非 data URL 防注入 + 守自包含）。**刻意不做**：参照的 agent 驱动多页 voice/tone 富化、`brand.json` 独立生命周期、渐进 skeleton 填充——我方 `from_url` 是快的单遍提取（voice&tone 仍进 DESIGN.md prose），无需渐进态；结构化多页富化是另一量级的 agent 子系统，越界。
- **B1-5 反爬协作式引导**：`from_url` 检测反爬（HTTP 403/429/503 + `looks_like_antiscrape` 挑战页特征——结构性标记全文匹配，`"just a moment"`/`"attention required"` 两个通用短语**仅在 `<title>` 内匹配**防误伤正文含该词的正常页）→ `ANTISCRAPE_HINT` 引导用户改用「从截图提取」（`from_image` 走视觉模型、绕过抓取层反爬）；前端 `runExtract` 优先展示后端可操作错误消息。**刻意延后**：browser 驱动 raw-CDP 取带样式 DOM（browser 后端仅暴露 ARIA 树非带样式 HTML，取样式 DOM 需 strict raw-CDP，单独设计）。
- **B2 引导式发现 UX**：`DesignChatPanel` idle + 末条 assistant 回复时于 composer 上方出 next-step chip（`DESIGN_NEXT_STEP_ACTIONS`：更精致/深色版/出个变体/质量评审，点击填 draft 不自动发）；头部「工具箱」按钮开 `DesignToolboxPopover`（可搜索、按形态分组的 recipes，点选组合起步 prompt 填 draft）。**刻意分歧**：空闲 placeholder 轮播（2-3）与「+」菜单设计上下文入口（2-4）需侵入**共享** `ChatInput`/`ComposerPlusMenu`（main+knowledge+design 三面共用），风险主对话回归；而 idle 发现已由空态 starter + next-step + 工具箱覆盖、当前产物上下文已每轮 `getExtraAttachments` 自动注入——判定不做共享 composer 手术。
- **B3 项目库 / 产物取出 / 版本历史**：
  - **3-1 项目库（`LaunchHome`）**：客户端标题搜索 + 网格/列表切换（`localStorage design:projects:view`）+ 卡片 hover more-menu（改名 Dialog / 复制 / 删除）+ 多选批量删（`selectMode` + `Set`，批量确认 AlertDialog + 父级 `Promise.allSettled` + partial 提示）+ **待复查徽标**（`needs_review_count` 项目级聚合，读取时 subquery COUNT `status='needs_review'`）。**刻意分歧**：参照做「网格/看板」（看板列按项目 status），我方**项目无 status 列**（status 只在产物级）→ 看板不适用，改「网格/列表」（对齐 roadmap「刻意不做 kanban」）。
  - **3-2 单产物取出**：单产物 ZIP 导出既有（导出菜单）；补**复制路径 + 在文件夹中显示**（`ArtifactView.artifact_path` + `tx.openFilePath`），**仅桌面**（`supportsLocalFileOps()` 且有 `artifactPath` 才显示，远端隐藏不破碎）。参照的 produced-file chip 本就扁平（折叠在 tool-action 级），我方保持扁平即忠实。
  - **3-3 版本历史双栏（`DesignVersionHistoryModal`，独立组件）**：左栏版本列表（溯源徽标 + 标题 + 相对时间 + `>3` 时搜索），右栏选中版本 **srcdoc live 预览**（沙箱 iframe，读磁盘 `versions/{n}/index.html` 快照，dormant bridge 未激活无害）；内容按版本号缓存 + hover 预取零往返。**溯源列**：`design_artifact_versions` 加 `origin`（`ai`/`manual`/`restore`）+ `prompt_summary`，在**全部 4 个 create_version 站点**赋值（生成/精修=ai、可视化编辑/换系统=manual、回滚=restore；`UpdateArtifactInput` 加同名字段缺省 ai）；分支未发布，CREATE TABLE + 幂等 ALTER 补列，无迁移。**恢复二次确认**（AlertDialog），恢复仍在后端生成**新**版本（原历史不动）。owner 新命令：`duplicate_design_project_cmd`（深拷贝产物磁盘目录 + 版本行 + 改写 artifact.json，失败整体回滚删项目 + 目录）、`get_design_artifact_version_html_cmd`（快照 HTML，Tauri `String` ↔ HTTP `Json<String>` 裸串对齐）。
- **B4 预览纵深（批注批量 / 设备视口 / 演示）**：
  - **4-2 批注批量带到对话**：`DesignCommentPanel` 多选 open 批注 → `handleBatchCommentsToChat` 合成一个 scope-guarded 结构块（编号 + 元素 snippet/tag + 反馈正文 + 「仅改这些」硬约束）作**单条 quote** 塞 composer（对齐参照 `<attached-preview-comments>`）。**刻意分歧**：参照 `applying/needs_review` 是绑 conversation-run 的瞬时态，我方就地精修静默单条 → 只保留 open/resolved（忠实终态），不引瞬时态。
  - **4-3 设备视口切换**：`PreviewDevice = auto|desktop|tablet|mobile`（`DEVICE_PRESETS` desktop 1440 / tablet 820×1180 / mobile 390×844）；`auto` 沿用产物自然 viewportW/H + 既有 zoom（**默认，零回归**），非 auto 走 `previewPaneRef` ResizeObserver 测面 → 整体 `deviceScale` 缩放适配 + 居中设备框（zoom 隐藏）。per-artifact `localStorage design:device:{id}` 记忆（参照用 session Map，我方按 roadmap 持久）。
  - **4-4 Present 演示**：`presentMode` 本窗口无 chrome 覆盖态（fixed inset-0 全屏 iframe + Escape 退出）+ `presentFullscreen`（`requestFullscreen` on `previewPaneRef`）。**刻意分歧**：参照有 `window.open(noopener)` 新标签，我方沙箱 srcDoc 无 URL → 不做（取出走 3-2 磁盘/导出）。
  - **4-1 绘制批注叠层（排为下一专项，不删）**：区域画框/涂鸦 + 烧进截图带到对话。最难/最脆：沙箱 opaque-origin 下捕获须改安全敏感 inspector bridge（SVG-foreignObject 光栅化脆弱）或 offscreen 重渲染（滚动错位）。现有元素钉 + 一键精修 + 4-2 批量已覆盖「标注→反馈」，4-1 差异=烧进截图，为守高可用/性能稳定单独立项。
- **B5 检视器编辑广度（链接 / 图片 / undo-redo）**：
  - **属性编辑引擎（`patch.rs`）**：`ElementPatch` 加 `attrs: Vec<(String,String)>`；新 `apply_attr_patch`（复用私有 `extract_attr`/`remove_attr`/`find_attr_pos` 逐属性重建 open tag、单次 splice）。**安全红线**：`ALLOWED_ATTRS = [href, src, alt]`——**只放行这三个**（绝不写 `onclick`/`onerror`/`style`，防事件注入）；`sanitize_attr_value` 拒 `javascript:`/`vbscript:`/`data:text/html`（去控制字符后判），`href` 拒任何 `data:`，`src` 仅放行 `data:image/*`，值经 `escape_attr`（`& < > "` → 实体，防击穿属性引号）；空值 = 清除属性。JS bridge `ds_preview_attr` 同持 href/src/alt 白名单。`find_attr_pos` **引号感知**（review 修复）：扫描跳过带引号属性值，避免值里 ` name=` 子串（如 `alt="见 src=x"`）被误命中致重复属性 + 编辑丢弃（style patch 同享此硬化）；仍要求 `name=` 紧邻（我方渲染器只产此形态，`href =` 空格式属已知低危边角、不处理）。
  - **`patch_element` 应用顺序（红线）**：text（改 inner，同 oid open tag 不移）→ **re-annotate** → attrs → **re-annotate** → styles。attrs 与 styles 同改一个 open tag，第一次改动后 open tag 字节范围移位，**必须 re-annotate 拿新 offset**（值仅变结构不变 → `annotate` 重赋同一 oid 序列，映射稳定）。
  - **bridge 属性回传**：`info(el)` 对 `<a>` 回传 `{href}`、`<img>` 回传 `{src,alt}`；inspector 据 `selected.tag` 显示链接段（href 输入）/ 图片段（src 输入 + 上传按钮 + alt 输入）。
  - **本地图 → data-uri（刻意分歧）**：参照本地上传写**项目相对路径**；我方产物须**自包含**（`image.rs` 本就内嵌 `data:image/*`）→ 忠实改为 data-uri：`imageToDataUri` fetch(src)→blob→canvas 降采样 + 700KB 字节预算（PNG 保透明 / 其余 JPEG），统一桌面（Tauri `convertFileSrc` 无 `File` 也可 fetch）与 HTTP（objectURL）。
  - **undo/redo（客户端 inverse-patch 栈）**：每次 commit 记 `{oid, before, after}`（before 取 `selected` 的当前/计算值），undo 回放 before、redo 回放 after，均经确定性 patch 引擎（切产物清栈，上限 50）。**串行化 + 提交成功才移栈**（`historyBusyRef` 防连按并发用同一 stale hash；副作用出 setState updater 防 StrictMode 双跑；commit 失败不移栈不脱节——review 修复）。inspector 草稿跟随**外部值变化**（防 undo 后失焦重提旧值抵消 undo）。`Cmd/Ctrl+Z` / `Cmd+Shift+Z`，焦点在 input/textarea/contenteditable 时让位原生撤销。**刻意分歧**：参照用 before/after 全源快照栈（且 UI 未渲染），我方前端无源、但 `selected` 持元素旧值 → inverse-patch 更轻、视觉等价、渲染 toolbar 按钮。
- **B6 生成前意图采集**：
- **6-1 discovery brief（`LaunchHome`，可选可跳过）**：折叠简报表单（受众 / 语气·风格 / 要点 / 参考），`composeBriefPrompt` 把已填字段拼成「设计简报」段并入生成 prompt（**空简报 = 原 prompt 逐字不变，零回归**；项目标题用干净 base 不用拼简报的 prompt，防漏进标题）。首页 recipe 点选以父级 `homeRecipeId` 为单一状态并回传 `LaunchHome`，卡片通过 `aria-pressed` + `bg-secondary/70` 持久显示选中，不能只填 prompt 而丢失可见状态；产物类型胶囊是强互斥生成目标，复用 `RadioPills variant="strong"` 的统一反白样式与 radio 语义，图标继承前景色且不改变边框。**刻意分歧**：参照用 LLM 授权的 `<question-form>` 协议（≈ 我方 `ask_user_question`），但那是**会话内阻塞工具**、绑 session、在聊天里渲染——不适配 LaunchHome 首屏直生成的 owner 面同步流；故取参照的**字段目录**（audience/tone/points/reference）+ 「可跳过 30 秒 brief」形态，落成同步 GUI 表单 + prompt 拼接（答案进 prompt 而非独立记录，与参照一致）。
  - **6-2 两段式 outline（刻意分歧：不做结构化门）**：参照对 deck **明确偏好一次成型**（`DECK_SKELETON_HTML` 填槽），outline-first **只在独立 Plan mode** 以软性可编辑 `.md` + 散文交接实现、**无结构化 outline→generate API**。我方 deck 亦一次成型（自包含 HTML + `<section class="ds-slide">`，一次成型是**强项**，不弱化）。故 6-2 忠实落在**对话式 outline-first**（设计对话新增「先规划大纲」starter，点选填 composer：让模型先出分节/分页大纲、确认后按大纲生成）——复用现有对话栈，**不新建结构化两段门**（那是超出参照的自研，违「源码级复刻」）。
- **B7 分享 / 部署 / deck 打印**：
  - **7-3 deck PDF 保真**：deck frame 加 `@media print`（`@page{size:1280px 720px}` 横版无边距、每张 `.ds-slide` 强制显示各占一页 `page-break-after:always`、隐藏 `.ds-deck-pager` chrome）；`PdfParams` 加 `prefer_css_page_size`（CDP + 扩展双 backend 透传），`render_native` 对 deck 传 `landscape + preferCSSPageSize` 让页内 `@page` 定纸张。**修真 bug**：此前裸默认 printToPDF=Letter 竖版只印首张 active 幻灯片。屏显行为零回归。**分辨率分歧**：参照 deck 用 1920×1080，我方沿用既有 1280×720（同 16:9，避免改动 authoring 分辨率的回归面）。
  - **7-1 只读分享（server 模式公开路由 + 桌面导出降级）**：`design_shares` 表（token PK + artifact_id FK CASCADE + 每产物唯一，幂等复用同一 token）；owner 命令 create/get/revoke（`/api/design/artifacts/{id}/share`，authed）；**公开无鉴权** `GET /api/design/share/{token}`（放 `health` 路由，与 `/api/health` 同层不进 `require_api_key`）→ token 白名单（≤128 纯字母数字，先校验后用）→ `render_share_html`（`render_clean` 干净自包含快照，无 bridge/oid）→ `sandbox allow-scripts` 隔离到 opaque origin（不能读服务端 cookie / 同源接口）+ no-referrer + nosniff；token=uuid v4 simple（32 hex，不可猜）。前端 Share 按钮：HTTP 模式建分享 + 复制 `origin/api/design/share/{token}`，桌面（无公开 server）导出干净 HTML 供发送（拍板降级）。**刻意分歧**：参照**无 token-share 服务**（「public = 部署 URL」`publicShareUrlForDeployment`）；但用户拍板「分享必做」独立于 opt-in 部署 = 要一条不依赖 CF 的低摩擦分享，故新建 server-mode token 快照（用户指令，非越参照自研）。
  - **7-2 Cloudflare Pages 部署（opt-in，`deploy.rs`）**：产物**自包含**故整站 = 单 `index.html` → CF 直传大幅简化（一 hash 一传）。序列对齐参照：ensure project → upload-token(JWT) → `blake3(base64+ext)[..32]` → check-missing → upload → upsert-hashes → create deployment(multipart manifest `{"/index.html":hash}`) → `<name>.pages.dev`。**安全红线**：① 所有出站 `guard()` 先过 `ssrf::check_url(url, Strict, ["api.cloudflare.com"])`（URL host 恒硬编码 `api.cloudflare.com`，`acct`/`name` 只进 path 不改 authority，guard 兜底内网/元数据 SSRF）；② API token **0600** 存 `credentials/cloudflare.json`（`platform::write_secure_file`），GUI 读经 `public_cf_config` **脱敏**（回 `hasToken` + mask 哨兵、绝不回明文），保存传 mask = 保留原 token；**属凭据平面、GUI-only、不进 `ha-settings`**（同 Provider API Key / MCP 凭据例外）；③ owner 命令显式触发，**后台自主维护绝不部署**。项目名 `ha-<slug>-<id8>`（DNS-safe ≤63，按产物稳定派生 → 重部署覆盖同子域）。UI `DesignDeployModal`（token+account 录入 + 部署 + 复制 URL）。单测锁 DNS-safe 名 / hash 确定性 / SSRF 拒内网。**刻意分歧**：参照支持 Vercel + CF 双 provider，我方按拍板**只做 CF Pages 单一 opt-in**（后续已补 Vercel 第二 provider，`deploy_vercel.rs`，同款凭据 / SSRF 红线）。
  - **部署就绪探测（`probe_deploy_ready` / `probe_design_deploy_cmd`）**：`*.pages.dev` / `*.vercel.app` 边缘传播有延迟，部署成功即甩 URL 时立即打开可能 404。owner 平面探测：`DesignDeployModal` 部署后自动轮询（至多 8×3s、token 化防错刷旧 URL），就绪前显示「链接生效中」徽章、停滞给「重新检查」。**SSRF 红线**：探测目标是用户**公网**站点（host 随部署商 / 自定义域可变），用 `SsrfPolicy::Default`（放行公网、拦私网 / 环回 / 元数据）+ **`reqwest::redirect::Policy::none()` 禁跟随跳转**——`check_url` 只校验首个 URL，若跟随默认 10 跳，公网 URL 可 `302→169.254.169.254/内网` 把探测变成盲 SSRF 内网扫描；就绪探测只需知「边缘有响应」（3xx 本身即 ready），故直接断跳转向量。
- **B8 媒体栈（音频）**：
  - **8-2 SFX 专用端点 + 时长（红线修真 bug）**：此前 ElevenLabs SFX 错误复用音乐端点 `/v1/music`（质量劣化）；改走**专用** `POST /v1/sound-generation`（模型 `eleven_text_to_sound_v2`，body `{text, duration_seconds[钳 0.5–30, 默认 5], prompt_influence 0.7}`，prompt 截 ≤450）。`AudioGenParams`/`generate_audio_parts`/`CreateArtifactInput` 加 `audio_duration_secs`（music 转 `music_length_ms`，sfx 钳 provider 区间）。`default_model` 按 kind 分离（speech/music/sfx 各自默认）。
  - **8-1 模型目录 + voices（已并入媒体生成子系统）**：预设模型/能力目录统一归 `media_gen/catalog.rs`（内置 vendor 模板单一真相源，GUI-only）；voices 归 `media_gen/voices.rs::list_media_voices`（按 provider_id 分派：elevenlabs `GET /v2/voices` 实时 + **10 分钟缓存按凭据 blake3 指纹（不缓明文 key）且按 provider_id 隔离**；openai 系静态表）。owner 命令 `list_media_voices` / `get_media_provider_templates`（Tauri + HTTP + COMMAND_MAP）。时长桶 `AUDIO_DURATIONS_SEC` 保留。t2v 按拍板不做（`video` 模态仅枚举预留）。详见 [media-generation.md](media-generation.md)。

---

## 10. 导出与产物库

### 10.1 导出格式（强路 + 客户端回退）

**PDF / PNG / 视频走「强路优先、客户端回退」两级**（`design/render_native.rs`）：

- **强路 = 真实浏览器原生捕获**：复用现有 CDP 浏览器后端（`crate::browser`）在隔离页（`new_page` → 捕获 → `close_page`，不碰用户标签）渲染产物 `index.html` →
  - **PDF** = `printToPDF`（**矢量、文字可选可搜**）
  - **PNG** = `captureScreenshot`（**全保真**，`backdrop-filter`/WebGL/真实字体全捕获，摆脱 html2canvas 的 CSS 子集天花板）
  - **视频（MP4）** = 注入确定性时钟 harness（与 `designVideo.ts` 同源）→ 逐帧 `__dsSeek` + 原生截图 → **ffmpeg** 编码 `libx264`。owner 入口 `export_design_native_cmd` / `GET /api/design/artifacts/{id}/native?format=pdf|png|video`。
- **两引擎均零内置、按需就位、跨环境自配**（**不打进安装包**，见 [§10.3](#103-导出引擎的按需配置)）：
  - **Chromium**：系统浏览器优先（`platform::find_chrome_executable` 探测 Chrome / Edge / Brave / Chromium，多数环境已装即用、零下载）→ 缺失才从 Google 快照 CDN 按需下载到 `~/.hope-agent/browser/`。
  - **ffmpeg**：`HA_FFMPEG_PATH` / PATH 优先 → 缺失才按需下载静态构建到 `~/.hope-agent/ffmpeg/`（macOS+Linux 用 martin-riedl.de、Windows 用 BtbN，各平台真实验证）。
- **客户端回退**：强路（无浏览器后端 / 无 ffmpeg / 失败）时前端自动降级——PNG/PDF 走 `html2canvas + jsPDF`，视频走 **WebCodecs**（`designVideo.ts`），始终可导出。

| 格式 | 强路 | 客户端回退 |
| --- | --- | --- |
| **HTML** | 直接产出 `index.html`（自包含内联，零依赖） | —— |
| **PNG** | `captureScreenshot`（全保真） | `html2canvas`（多页 deck 纵向拼图） |
| **PDF** | `printToPDF`（矢量可选文字） | `html2canvas + jsPDF`（位图） |
| **视频 MP4** | 逐帧真渲染 + ffmpeg（任意时长/分辨率、跨浏览器无关） | WebCodecs 客户端逐帧编码 |
| **PPTX** | 前端整页栅格化 + 后端 `zip`+OOXML 组装 | （同左，PPTX 无强路） |
| **ZIP / Markdown** | 后端打包 / `htmd` 转换 | —— |
| **代码交付包（工程轴 C）** | 后端 `export_handoff` 打包 | —— |

**图片导出格式 / 清晰度就地选**：导出菜单除快捷「PNG 图片」（走上表全保真强路）外，另有「图片（格式 / 清晰度）」轻量弹窗——**PNG/JPEG × 1/2/3x 就地选**。显式选项走**客户端栅格化**（`designExport::exportPng` + `canvasToImageBlob` 按 `format` 出 PNG/JPEG、`scale` 任意倍率）而非原生强路（native 只出默认 PNG，无法表达 JPEG / 自定义倍率）；快捷 PNG 保持原生强路不退化。文件名扩展名随格式（`.png`/`.jpg`）。

`exports/` 目录**必须 gitignore**（restore 会清）；HTTP 导出路由需 `DefaultBodyLimit` 放开。

**保存出口统一走 `Transport.saveFileAs`（选目录 + 存后 reveal，桌面 / HTTP 不对称）**：所有导出字节（HTML/MD/PNG/PDF/PPTX/MP4/ZIP/代码交付包/Token 代码/DESIGN.md）不再各自 `downloadBlob` 直落浏览器下载目录，而是经单一出口 `tx.saveFileAs(blob, filename) → SaveResult`，前端 `presentSaveResult` 统一收敛提示：
- **桌面（Tauri）**：`@tauri-apps/plugin-dialog.save()` 原生「保存到…」框（`defaultPath` 预填 **上次导出目录**，localStorage `design_export_dir` 记忆）→ 用户选定后经 `save_exported_file`（base64→写盘，见 [api-reference](api-reference.md)）落盘 → toast 带「**在文件夹中显示**」动作（`invoke("reveal_in_folder")` 高亮该文件）。取消保存框 = 静默关 loading，不当失败。
- **HTTP / 网页**：优先 `showSaveFilePicker`（File System Access，仅 Chromium + 安全上下文）让用户选目录；不支持（Firefox / Safari / 非 https）自动回退 `downloadBlob` 浏览器下载。**故意纯客户端**——远端**绝不写服务器磁盘**（`save_exported_file` 无 HTTP 路由），沙箱也无法 reveal，`SaveResult.path=null`。

`downloadBlob` 抽到 leaf module `src/lib/fileDownload.ts`（避免 `transport-http` → `designExport` → `transport-provider` 导入环）；`designExport` re-export 保持旧调用点不变。

**代码交付包（developer handoff，工程轴 C）**：`service::export_handoff(artifact_id)` 把产物打成一个面向工程侧的 ZIP——`index.html`（`editable=false` 干净渲染）+ `source/`（body/css/js）+ `tokens/`（复用 [§6.7](#67-多平台-token-导出designtokenexport) 的 `token_export::export_all`，六平台代码）+ `HANDOFF.md`（形态 / 设计系统 / **本产物实际引用的 `var(--ds-*)` 变量清单**——`referenced_tokens` 精确边界匹配避免 `--ds-color` 误命中 `--ds-color-primary`）。owner 平面 `export_design_handoff_cmd`（Tauri）/ `GET /api/design/artifacts/{id}/handoff`（HTTP，base64）。这是把「设计产物 + 设计系统」一次性交到工程手里的闭环。

### 10.2 产物库

统一缩略图墙（跨项目/项目内）+ 版本对比（并排 iframe / 缩略图 diff）+ 批量导出 + 分享入口。owner 平面读 `list_artifacts` / `get_artifact`，取消/删除复用统一入口。

### 10.3 导出引擎的按需配置

强路依赖两个原生引擎（Chromium 渲染、ffmpeg 编码），二者都**不打进安装包**，而是首次需要时就位——目标是「各环境开箱即用，且永不因缺引擎而卡死」。前端在导出前经统一 gate（`DesignView` 的 `exportGate`）先探状态，再决定直接导出 / 引导下载 / 客户端回退。

- **两级 doctor 三态**：`ffmpeg::doctor()` 与 `render_native::browser_export_status()` 各返回 `{ ready, source, binary_path, can_auto_install }`。`source` 区分 `env`/`path`/`runtime`/`missing`（ffmpeg）与系统浏览器/已下载 runtime/`missing`（Chromium），让 UI 精确提示。视频导出**同时**预检两引擎（缺任一即引导），避免下了 Chromium 才发现没 ffmpeg 的二次中断。
- **Chromium 就位**：系统浏览器优先（`platform::find_chrome_executable`，macOS `.app` / Linux `which` / Windows `ProgramFiles`·`LOCALAPPDATA`，覆盖 Chrome / Edge / Brave / Chromium）→ 缺失才复用 `browser::runtime`（Google `chromium-browser-snapshots` CDN，每平台 pin 版本）按需下载。
- **ffmpeg 就位**（`crate::ffmpeg`，镜像 `browser::runtime` 的信任模型）：`HA_FFMPEG_PATH` / PATH 优先 → 缺失才按需下载**静态 zip 构建**。**两源分治**——macOS(arm64/amd64) + Linux(amd64/arm64) 用 martin-riedl.de（单文件 `ffmpeg` 在根）、Windows(amd64) 用 BtbN `win64-gpl`（二进制嵌套 `…/bin/ffmpeg.exe`），故 `FfmpegSpec.binary_relpath` 逐平台不同。下载走**重试 + HTTP `Range` 续传**（3 次尝试、指数退避、短读守卫、体积上限），extract **只取目标二进制**（跳过 Windows 包内 ffplay/ffprobe，省 ~290 MB），落盘后 `-version` 冒烟测试通过才原子提升为 ready。SSRF 走 `security::ssrf::check_url`（固定构建主机）。
- **失败即降级、绝不卡死**：任一引擎下载 / 解压 / 冒烟失败一律返回 `Err`，导出流程降级到「引导安装 + 客户端回退（html2canvas/jsPDF/WebCodecs）」，**永不 panic、永不白屏**。进度经 EventBus `design:ffmpeg_download_progress` / `browser:chromium_download_progress` 上报，UI 实时展示。

---

## 11. 质量评审门与设计方向选择器

### 11.1 5 维质量门

`design(action="critique", artifact_id | html)` 走 [side_query](side-query.md)（复用主 system prompt 前缀命中 cache，成本低）对产物做 5 维评审：**品牌契合 / 可访问性(a11y) / 视觉层次 / 可用性 / 性能**，返回每维评分 + 具体可执行修复 + 总分。可配 `auto_critique` 在 finalize 前自动跑（反 AI-slop：占位内容 / 变体雷同 / 对比度不足）。总分落 `critique_score`（版本级）。

### 11.2 设计方向选择器

brief 缺设计系统时，`design(action="propose_directions", brief)` 返回 N（默认 4）个方向选项（每个是一份 mini 设计系统预览：色板 + 字体 + 一个样例组件）。前端渲染为可选卡片，用户选定即作为该产物/项目的设计系统；也可"从截图/URL 导入"走 D2 反向提取。

### 11.3 反 AI-slop 确定性自查（`self_check`）

与 §11.1 的 LLM 评审互补的**确定性、无 LLM** 质量闸（`design::selfcheck`，`design.self_check` 门控、默认开）。两类单产物信号：**thin**（剥掉 `<script>`/`<style>`/注释后元素开标签与可见文字都低于下限 = 近空壳）与 **placeholder**（命中高置信占位/填充标记，如 `lorem ipsum` / `your text here` / `#REPLACE_ME`）。命中即在创建 / 生成定稿 / 编辑落版本时翻 `needs_review` 并把 `selfCheck` 键**合并**进 `metadata`；正文改好或关闭开关后清键回 `ready`（**只回收自动标记**，不覆盖其它 metadata）。另有 `near_identical`（去标签后可见文字的**字符 5-gram** shingle Jaccard；CJK 无词边界故用字符级）供多方向候选去雷同复用。**刻意从严**——阈值只抓近空壳 / 高置信占位，避免误标合法产物。区别于两个竞品的质量闸（均为 LLM 自判：成本高、非确定）：本闸 LLM-free、确定、可单测，是差异化护城河。

---

## 12. 与现有子系统的契约

- **[知识空间](knowledge-base.md)（D4）**：`save_to_knowledge` 生成 KB 笔记内嵌产物预览链接 + 元数据 → 设计产物进第二大脑可检索；读取即 untrusted 信封约束不变。
- **[项目](project.md)（D4）**：设计项目经**双源代码仓库绑定**（`code_dir` 本机目录 / `ha_project_id` HA 项目派生，见 §6.9）关联真实代码工程；`extract_system from=codebase` 的 agent 面读根 = 会话工作目录 ∪ 附件 ∪ 绑定仓库（`scoped_local_path` fail-closed，**非** `WorkspaceScope`——那是文件浏览器的作用域原语）。
- **[系统提示词](prompt-system.md)（D4）**：会话可附着一个设计系统，`design` prompt 段以名称 + 气质摘要注入（预算受控、静态 prefix cache 友好），像[记忆](memory.md)/知识那样约束生成；incognito 零注入。
- **[媒体生成](media-generation.md)**：`image` / `audio` kind 复用统一媒体生成栈（`media_gen::execute_image/execute_audio`），不重造。
- **[side_query](side-query.md)**：质量门 / 反向提取 / 方向选择器的 LLM 评审走 side_query 降本。
- **[工具系统](tool-system.md) / [权限](permission-system.md)**：`design` 工具 `internal`；涉及外部工程写入的 action owner 专属、不进 agent schema。
- **[会话](session.md)无痕**：incognito 会话零设计注入、跳过自动沉淀、产物不进全局索引（对齐关闭即焚）。
- **[后台任务](background-jobs.md)**：生成/导出/提取标 `async_capable`，走 `JobManager` 统一后台模型（不起平行 API）。

---

## 13. 权限 · 安全 · 沙箱 · 无痕（红线）

| 风险 | 缓解 |
| --- | --- |
| 产物脚本访问主应用 DOM / cookie | iframe `sandbox="allow-scripts"`（无 `allow-same-origin`），只能 postMessage |
| 路径穿越读凭据 | 静态托管三闸：`^[A-Za-z0-9_-]{1,128}$` id 白名单 + `validate_safe_rest_path`（拒 `..`/反斜杠）+ `contained_canonical`（canonicalize 后断言子树包含） |
| 沙箱消息伪造 → 恶意写盘 | 父窗数值净化 + 白名单令牌 + 破坏字符转义 + `expected` stale-write（见 §7.3） |
| `extract_system from=url` SSRF | 出站必过 `security::ssrf::check_url`，禁自写 IP 校验 |
| 后台自主维护写外部工程 | 一律拒（对齐知识空间外部只读红线），提取默认落 managed |
| 凭据泄漏进产物 / 导出 | 日志 `redact_sensitive`；产物/系统模板本身不写凭据 |
| incognito 泄漏 | 无痕会话零注入、不沉淀、产物不进全局索引 |
| HTTP 模式任意主机路径读 | 导出/预览按路径读须校验落在设计目录子树内，远端拒任意主机路径 |

写盘一律走 `crate::platform::write_atomic`（temp+fsync+rename，禁回退 `fs::write`）。

---

## 14. 配置（设置三件套）

`AppConfig.design`（`design::DesignConfig`）：

| 字段 | 默认 | 含义 | 风险 |
| --- | --- | --- | --- |
| `enabled` | `true` | 全局开关 | LOW |
| `auto_show` | `true` | `create_artifact` 后自动聚焦预览 | LOW |
| `default_system_id` | `null` | 新产物默认设计系统 | LOW |
| `auto_critique` | `false` | finalize 前自动跑质量门 | MEDIUM |
| `max_versions_per_artifact` | `50` | 单产物保留版本数 | LOW |
| `panel_width` | `480` | 面板默认宽度 | LOW |
| `self_check` | `true` | 反 AI-slop 自查 | LOW |

三件套：GUI [`DesignSettingsPanel.tsx`](../../src/components/settings/) + [`tools/settings.rs`](../../crates/ha-core/src/tools/settings.rs) `design` category（含 `core_tools.rs` enum）+ [`skills/ha-settings/SKILL.md`](../../skills/ha-settings/SKILL.md) 风险登记。写配置走 `mutate_config(("design", source), …)`（不走 canvas 老 API 的 lost-update 覆辙）。

---

## 15. HTTP 路由与 Tauri 命令对照

每个能力同时暴露 Tauri IPC 与 HTTP，业务逻辑统一在 `ha_core::design::service`。（详表随实现填入 [api-reference.md](api-reference.md)。）

| 能力 | Tauri 命令 | HTTP 路由 | Transport key |
| --- | --- | --- | --- |
| 列出项目 | `list_design_projects_cmd` | `GET /api/design/projects` | 同名 |
| 项目 CRUD | `create/update/delete_design_project_cmd` | `POST/PUT/DELETE /api/design/projects[/{id}]` | 同名 |
| 列/取/删产物 | `list/get/delete_design_artifact_cmd` | `GET/DELETE /api/design/projects/{pid}/artifacts[/{aid}]` | 同名 |
| 建/流式生成产物 | `create/generate_design_artifact_cmd` | `POST /api/design/artifacts[/generate]` | 同名（generate 返 generating 壳、内容走 `design:generate_delta`，见 §5.6） |
| 版本/恢复 | `design_artifact_versions/restore_cmd` | `GET/POST …/artifacts/{aid}/versions` | 同名 |
| 可视化回写 | `design_patch_element_cmd` | `POST …/artifacts/{aid}/patch` | 同名 |
| 设计系统 CRUD | `list/get/save/rename/delete_design_system_cmd` | `GET/POST/PATCH/DELETE …/api/design/systems[/{id}]`（`rename` = `PATCH {name}`，内置拒改） | 同名 |
| 反向提取 | `design_extract_system_cmd` | `POST /api/design/systems/extract` | 同名 |
| 导出 | `design_export_cmd` | `POST …/artifacts/{aid}/export` | 同名 |
| 部署（CF/Vercel）+ 就绪探测 | `deploy_design_artifact[_vercel]_cmd` / `probe_design_deploy_cmd` | `POST …/artifacts/{aid}/deploy[/vercel]` · `POST /api/design/deploy/probe` | 同名（probe body `{url}`→`{ready,status}`，见 §B7-2） |
| 质量门 | `design_critique_cmd` | `POST …/artifacts/{aid}/critique` | 同名 |
| 缩略图 | `design_save/get_thumbnail_cmd` | `…/artifacts/{aid}/thumbnail` | 同名 |
| snapshot 回传 | `design_submit_snapshot_cmd` | `POST /api/design/snapshot/{requestId}` | 同名 |
| 静态托管 | （Tauri `asset://` 直读） | `GET /api/design/projects/{pid}/artifacts/{aid}/{*rest}` | iframe 直连 |
| 配置读写 | `get/save_design_config_cmd` | `GET/PUT /api/config/design` | 同名 |
| code→design 回灌 | `design_check_code_drift_cmd` / `design_code_drift_changes_cmd` / `design_code_drift_sync_cmd` | `POST …/projects/{pid}/code-drift/check` · `GET …/artifacts/{aid}/code-drift` · `POST …/artifacts/{aid}/code-drift/sync` | 同名 |
| 最近查看上报 | `mark_design_artifact_opened_cmd` | `POST …/artifacts/{aid}/opened` | 同名（MCP active-context 事实源） |

### 15.1 分发面（工程轴对外集成）—— HTTP 分发面 + 平台级 `hope-agent mcp`（design 首个 provider）

外部编码 agent / 脚本要复用本设计引擎（生成产物、反查设计系统、导出交付包、部署），**远程分发面是上表这套已完整的 `/api/design/*` HTTP 表面**——在 server 模式（`hope-agent server`）下经 Bearer Token 鉴权全量可用。典型集成链：`extract_system` → `create/generate_artifact` → `critique` → `export`/`handoff` → `deploy` → `share`。

**平台级 MCP 已落第一块砖**：本机外部 agent（Claude Code / Cursor）经标准 MCP 驱动走 `hope-agent mcp` stdio server——共享 host 在 `ha-core/src/mcp_server/`（`ToolProvider` 注册表 + JSON-RPC 循环 + 写门），design 经 `design/mcp_provider.rs` 挂入为**首个 provider**。**红线不变——依然不做 design 专属 MCP server**（把 Hope Agent 暴露成 MCP server 是平台议题，design 只是 provider、不自起 server）；`knowledge-mcp` 已发布子命令保持原样（内部归并共享循环记 P+1）。写门：默认只读，`--allow-writes` 才暴露写工具集；**恒不暴露** `implement_to_code` / 代码绑定写 / `deploy` / `share` / `delete_project` / `delete_artifact` / `save_to_knowledge` / `extract_system`（无会话无法安全界定读根）/ `export_*`——外部 agent 不得经 MCP 写用户代码仓库、对外发布或删除容器。协议 / 工具表 / active-context 机制见 [`mcp-server.md`](mcp-server.md)。

**Backlog**：设计系统包从 GitHub / 本地目录导入（安全安装路径）不依赖 MCP，独立推进。

---

## 16. 文件清单（注册触点）

新增平级子系统的注册触点（坐标来自现网 knowledge 子系统对照）：

### 后端

| 文件 | 角色 |
| --- | --- |
| `crates/ha-core/src/design/{mod,service,db,renderer,system,patch,critique,export,recipe}.rs` | 核心：注册表 + 业务 + 渲染 + token 编译 + oid 回写 + 质量门 + 导出 + 模板 |
| `crates/ha-core/src/design/{code_sync,code_watcher}.rs` | code→design 回灌：回执/收割/漂移检测 + notify 文件监听 |
| `crates/ha-core/src/design/mcp_provider.rs` | design 的 MCP `ToolProvider`（平台 `hope-agent mcp` 首个 provider） |
| `crates/ha-core/src/mcp_server/mod.rs` | 平台级 MCP server host（stdio + ToolProvider 注册表，见 mcp-server.md） |
| `crates/ha-core/src/tools/design/mod.rs` | `design` agent 工具（多 action 路由） |
| `crates/ha-core/src/lib.rs` | `pub mod design;` + `pub mod mcp_server;` |
| `crates/ha-core/src/paths.rs` | `design_dir` / `design_*_dir` |
| `crates/ha-core/src/config/mod.rs` | `AppConfig.design` |
| `crates/ha-server/src/routes/design.rs`（+ `routes/mod.rs` `pub mod` + `lib.rs` `.route`） | HTTP 薄壳 + 静态托管 |
| `src-tauri/src/commands/design.rs`（+ `commands/mod.rs` `pub mod` + `lib.rs` `generate_handler!`） | Tauri 薄壳 |
| `design-assets/{systems,recipes}/` | 内置设计系统与模板（随 App 发行） |

### 前端

| 文件 | 角色 |
| --- | --- |
| `src/components/design/DesignView.tsx` | 独立视图外壳（Home ↔ Studio） |
| `src/components/design/DesignHome.tsx` | 首页（prompt-first + 类型卡 + 最近项目墙） |
| `src/components/design/DesignStudio.tsx` | 工作室（产物库 + 单产物预览 + 检视抽屉 + AI 面板） |
| `src/components/design/DesignInspector.tsx` | 属性检视器（8 分区控件：文本/颜色/排版/间距/布局/尺寸/描边/效果；live 预览 + commit 回写） |
| `src/components/design/DesignChatPanel.tsx` | AI 对话面板（复用 `useChatStream`） |
| `src/components/settings/DesignSettingsPanel.tsx` | 设置 GUI |
| `src/App.tsx` | `view` 联合 + `lazy` + 渲染分支 + `onOpenDesign` prop |
| `src/components/common/IconSidebar.tsx` | 「知识空间」下方入口按钮 + props |
| `src/lib/transport-http.ts` | `COMMAND_MAP` 加 `*_cmd → path` |
| `src/lib/designRuntime.ts` | inspector bridge / oidmap 前端侧纯逻辑 |
| `src/types/design.ts` | 类型定义 |
| `src/i18n/locales/*.json` | 顶层 `design` 命名空间（12 语） |

---

## 17. 命名与关键设计决策

- **产品名"设计空间"**：与"知识空间"平级对仗；代码标识 `design`；无任何外部参考实现名。
- **推倒重做而非改进 atelier**：用户明确 atelier"做得不好"要求重做；三大痛点（画布卡 / 渲染重白屏 / 微调不好用）由本版三条架构原则逐条对症（轻量自包含 HTML / 无画布产物墙 / 纯 HTML 确定性回写）。
- **轻量 vs 重量运行时**：本版选**自包含 HTML + iframe 直载**（对齐 agent 原生设计工作空间品类主流做法），拒绝旧版的浏览器内 React/esbuild-wasm/Tailwind 编译——这是"不白屏、启动快、微调稳"的根本保证。
- **文件即真相源**：磁盘存正文，`design.db` 可重建索引。
- **内置设计系统两类**：6 套原创原型语言 + 一批品牌风格参考（种子展开、渲染附免责声明、非官方）。品牌参考仅作对公开视觉语言的独立再诠释，商标归各自所有者。
- **四大差异化全做**：D1 可视化微调（架构层做扎实）/ D2 本地反向提取护城河 / D3 一键导出与产物库 / D4 知识空间与项目联动。
