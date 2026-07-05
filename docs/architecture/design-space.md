# 设计空间（Design Space）子系统架构文档

> 返回 [文档索引](../README.md)
>
> 设计空间是 Hope Agent 的 **agent 原生设计工作空间**：用户与模型协作，从一句话或参考图产出**自包含、可交付的设计产物**（网页 / 移动原型 / 演示文稿 / 仪表盘 / 海报 / 文档 / 邮件 / 图像），以可复用的**品牌设计系统**为底座，在沙盒面板实时预览、可视化直接微调、版本管理、一键导出，并与[知识空间](knowledge-base.md)、[项目](project.md)深度联动。侧边栏入口紧贴「知识空间」下方。
>
> 产品名 **设计空间**；代码标识 `design`（模块 `crates/ha-core/src/design/`、agent 工具 `design`、数据库 `design.db`、前端视图 `DesignView`、右侧面板与 i18n 命名空间 `design`）。产品名与代码标识**均不引用任何外部参考实现的名称**。
>
> **迭代计划见** [`design-space-roadmap.md`](design-space-roadmap.md)（分期 / 工作流 / 验收标准 / 决策账本）。本文是子系统设计与实现的单一真相源；跨 PR 必守的红线摘要另见 [AGENTS.md](../../AGENTS.md)。

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

设计空间让模型与用户协作，从一句话或参考图产出**成体系、可交付的设计产物**，落在一个稳定、快速、可视化可编辑的工作台里，并可一键导出与沉淀。它对标 agent 原生设计工作空间这一品类，覆盖其全部产物形态与设计系统机制，并在四个方向上做出超越（见 [§2](#2-核心竞争力四大差异化)）。

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
9. **原创设计语言，零抄袭痕迹**：内置设计系统是**原创的、原型化的**设计语言（极简现代 / 编辑杂志 / 科技暗色 …），**不克隆任何真实品牌**（既规避商标风险，也彻底消除抄袭痕迹）。代码 / 注释 / commit / 文档 / UI / i18n 均不出现任何外部参考实现的名字。

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
| **产物版本（Version）** | 一次 update / restore / 可视化编辑产生的源码快照 | 递增；超 `maxVersionsPerArtifact` 按版本号倒序保留最新 N |
| **设计系统（DesignSystem）** | 可复用品牌契约：`DESIGN.md`（9 段，真相源）+ `tokens.json`（解析缓存） | 内置只读 / 用户创建 / 反向提取；套用到产物即注入 `:root` token |
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
    ha_project_id TEXT,                -- 弱引用 Hope Agent 项目（D4 联动）
    session_id TEXT, agent_id TEXT,    -- 弱引用来源（无 FK）
    created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
    metadata TEXT                      -- 预留 JSON
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
    created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
    metadata TEXT
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
| `deck` | 演示文稿 | 1280×720 (16:9) | 自带 `<section>` 翻页器（←/→/Space、页码），一份文件多页 |
| `dashboard` | 数据仪表盘 | 1440×自适应 | 网格布局容器 |
| `poster` | 海报 / 社媒图 | 尺寸预设（1080×1080 / 1080×1920 / A4 …） | 定尺容器 |
| `document` | 文档 / 规格 / 报告 | 页宽阅读容器 | 目录 + 排版 |
| `email` | 营销邮件 | 600 宽 | table 回退兼容 |
| `image` | 图像 | —— | 复用 `image_generate`，产出栅格图，不走 HTML 骨架 |

`image` 是唯一非 HTML 产物：它复用现有[图片生成](image-generation.md)子系统，把结果落进产物目录并登记，参与同一产物库 / 版本 / 导出（导出即原图）。

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

`system::compile_tokens(system_md) -> tokens.json`：从 DESIGN.md 结构化区块解析出 CSS 自定义属性（`--ds-color-primary`、`--ds-space-4`、`--ds-font-sans`、`--ds-radius-md` …）。渲染时 `renderer` 把 `tokens.json` 展开为 `:root { … }` 注入产物。产物 CSS 引用变量而非硬编码 → **套用/切换设计系统即换皮，一致性由 token 锁定保证**。token 另可导出为 CSS / TS / DTCG（Phase 6 可选）。

### 6.3 内置设计系统（原创原型语言 + 品牌风格参考）

两类随 App 发行，都是完整 DESIGN.md + token，用户可 fork / 反向提取新建：

- **6 套原创原型语言**（`system.rs::builtins`）：极简现代、编辑杂志、科技暗色、温暖亲和、专业金融、大胆活力，覆盖常见气质光谱。
- **一批品牌风格参考**（`brands.rs` 的 `BrandSeed` 种子 → `system::expand` 展开为完整 25 token 契约）：覆盖开发者工具 / AI / SaaS / 设计框架 / 社交 / 媒体电商 / 大厂等主流品牌。每个种子只声明签名色 / 字体 / 圆角 / 字号密度 / 气质，`expand` 按背景明暗自适应补齐语义色 / 中性色 / 阴影，保证 token 契约齐全一致。**均为对各品牌公开视觉语言的独立再诠释，仅供设计参考**——`build_system_md` 对 `brand_ref=Some(..)` 的系统在摘要下自动附一行免责声明（非官方、无隶属 / 赞助 / 授权、商标归各自所有者），原创系统不附。

**分组与选择（`category` 字段）**：`BrandSeed` 按分节经 `brands.rs::cat(..)` 统一打上类目（开发者工具 / AI 产品 / SaaS / 设计框架 / 社交 / 媒体电商 / 大厂），原创系统类目为「原创原型」；`category` 落 `design_systems` 表（旧库启动期幂等 `ALTER TABLE` 补列 + `backfill_system_category` 仅填 NULL）随 `list_systems` 返回。GUI 侧 `DesignSystemPicker`（Dialog + 搜索框，规避菜单内输入焦点冲突）按 `category` 分组、按 name/summary/category 即时过滤；DesignView 头部与设置页「默认设计系统」共用。用户自建 / 提取系统 `category=None`，归「我的设计系统」组。

### 6.4 反向提取（D2 护城河）

`design(action="extract_system", from, ref)`：

- `from=image`（截图 / 设计稿）→ 多模态 LLM 分析视觉 → 生成 `DESIGN.md` + `tokens.json`
- `from=url` → `security::ssrf::check_url` 后抓取页面 + 首屏截图 → 提取
- `from=codebase`（本地代码工程）→ 读工程的 CSS / tailwind config / design token 文件 / 现有 `DESIGN.md` → 归纳 `DESIGN.md`

**owner 写入为主**：反向提取默认落 managed 设计系统目录（用户可见可编辑），**后台自主维护绝不写外部工程**（对齐知识空间外部只读红线）。

### 6.5 DESIGN.md 规范互通（导入 / 导出）

`DESIGN.md` 既是内部落盘格式，也是**跨工具互通格式**：

- **导入**（`service::import_design_md` / 工具 `design(action="import_design_md", content)` / owner `POST /api/design/systems/import`）：解析任意 DESIGN.md——`design_md::extract_tokens` 从 `:root{}` / 表格 / 内联抽 `--ds-*` token（≥4 个即确定性直用，**零 LLM 成本**）；token 不足时用 LLM 从正文合成，但**始终保留原 DESIGN.md 正文**（不改写用户 prose）。source 记 `imported`。
- **导出**（`service::export_design_md` / 工具 `design(action="export_system")` / owner `GET /api/design/systems/{id}/design-md`）：`design_md::to_design_md` 输出正文 prose + 末尾 Token 表，**可无损再导入**。
- **`from=codebase`** 反向提取本就读工程内现有 `DESIGN.md`，与导入互补。

---

## 7. 可视化直接微调（选中→反查→回写）

这是 D1，也是旧版做不好、本版从架构层解决的能力。**根因分析**：旧版产物是编译后的 React 组件，用户在 DOM 上的改动要反查回 JSX 源，中间隔着 React 渲染与 Tailwind 编译，映射有损、经常改不动或改坏。**本版产物是纯 HTML，渲染 DOM 与源码结构一一对应，回写是确定性的。**

### 7.1 oid 映射（渲染期建立）

`renderer` 在编译产物时，遍历 body HTML 的每个元素，注入 `data-ds-oid="{n}"`（源码文档顺序编号），同时产出 `oidmap.json`：`{ "12": { start: 3480, end: 3560, tag: "button", ... } }`（源码字节范围）。oidmap 随版本落盘。

### 7.2 交互三通道

1. **对话改写**（自然语言）：让 AI 改，产出新版本，可要多个变体并排。
2. **就地直接编辑**（选中产物进入编辑态）：
   - 点选元素 → bridge postMessage `{oid, tag, computedStyle, textContent, rect}` → `DesignInspector` 显示**分区控件**（文本 / 颜色 / 间距 / 排版 / 尺寸 / 填充 / 描边）。
   - 改控件 → bridge 即时把 inline style / 文本应用到 live DOM（**零延迟乐观预览**）→ 交互结束 commit：owner 端 `patch_element(artifact_id, oid, patch)` 按 oidmap 定位源码字节范围，确定性回写（生成新版本）。
   - 文本双击 → contenteditable → commit 写回文本节点源码范围。
3. **批注钉**：点击元素留批注（结构化上下文：产物 + 选择器 + 元素摘要）回灌对话让 AI 精修；批注可标记已解决。

### 7.3 回写红线（对齐知识空间 / atelier 已验证的安全点）

- **沙箱消息不可信**：iframe → 磁盘写是首个不可信写通道。文本/样式落盘前，父窗做数值净化（NaN/极值拒）、白名单式令牌色板（排除表正则永远列不全，只允许白名单）、JSX/HTML 破坏字符转义。
- **确定性命中**：`patch_element` 按 oidmap 字节范围唯一命中；命中 0 处或源已变（`expected` hash 不符）→ 拒绝（stale-write 守卫），前端提示"源已更新，请重新选中"。
- **撤销/重做 + 版本**：每次 commit 建新版本，可 restore。
- **单一稳定 iframe**：编辑面是一个固定 iframe，无画布 transform，无 pointer capture 自研逻辑 → 无卡顿、无拖拽泄漏。

---

## 8. Agent 工具面（`design` 工具）

单一 `design` 工具（`internal: true`，按 action 路由；生成类 `async_capable`），供模型自主创建/迭代设计。

| Action | 语义 | 平面 |
| --- | --- | --- |
| `list_systems` / `get_system` | 浏览 / 读取设计系统契约（供生成时 grounding） | agent |
| `list_recipes` / `get_recipe` | 浏览 / 读取产物模板指令 | agent |
| `create_system` / `extract_system` | 新建 / 反向提取设计系统 | agent（提取默认落 managed，不写外部工程） |
| `list_projects` / `list_artifacts` / `get_artifact` | 浏览项目与产物 | agent |
| `plan_artifacts` | 声明批量生成规划（出骨架卡） | agent |
| `create_artifact` | 生成产物（kind + system + html/css/js），渲染 + 预览 | agent |
| `update_artifact` | 迭代产物（新版本 + reload） | agent |
| `patch_element` | 按 oid 定向回写（可视化编辑复用同一后端） | owner + agent |
| `delete_artifact` / `versions` / `restore` | 删除 / 版本列表 / 恢复 | agent |
| `snapshot` | 渲染 PNG（缩略图 + 多模态自反馈回路） | agent |
| `critique` | 5 维质量门评审 | agent |
| `propose_directions` | 无设计系统时给 N 个方向选项 | agent |
| `export` | 导出 HTML/PDF/PPTX/PNG | owner + agent |
| `save_to_knowledge` | 沉淀产物为 KB 笔记（D4） | agent |

**关键不变量：**

- **kind 不可变**：`update` 沿用 `create` 时的 kind，换类型只能删建。
- **snapshot 自反馈**：`snapshot` 返回值走 `IMAGE_BASE64_PREFIX`（与 browser/canvas 共用），执行层物化为多模态 image 输入 → 模型能"看到"自己的设计并迭代。
- **owner 覆盖不进 agent schema**：涉及外部工程写入 / 权限的动作是 owner 专属（模型 schema 不暴露），防注入提权。

---

## 9. 前端视图与工作台

蓝本参考 [`KnowledgeView.tsx`](../../src/components/knowledge/KnowledgeView.tsx) 的 Header + 多栏可拖拽可折叠骨架，但**更简单、更稳**（无画布）。

### 9.1 三层结构

- **首页 `DesignHome`**：顶部大输入框（prompt-first，一句话起步直达生成）+ 产物类型卡（web/mobile/deck/dashboard/poster/document/email/image）+ 最近项目**缩略图墙**（纯 CSS grid，无画布）。
- **工作室 `DesignStudio`**：
  - 左：产物库（缩略图 grid / 列表切换）+ AI 对话面板（复用主对话 `useChatStream`，同知识空间 `KnowledgeChatPanel` 模式）。
  - 中：**单产物聚焦预览**——一个稳定 iframe，顶部工具条（缩放下拉：50% / 100% / 适应宽度；刷新 / 全屏 / 导出 / 分享），纯 CSS `transform: scale()` 缩放，**无平移画布**。
  - 右：**检视抽屉**（选中元素/产物时滑出）：属性 / 代码 / 设计系统 / 批注 四页签。
- **多产物概览**：产物库即概览，点缩略图聚焦。**不做无限画布**。

### 9.2 状态与交互

纯 `useState`（对齐 KnowledgeView，无 Redux）；栏宽/折叠 localStorage 持久化；`getTransport().call("*_cmd")` 与后端交互；`tx.listen("design:*")` 增量刷新。会话切换时 transient UI 状态（全屏/缩放）强制清零。

### 9.3 侧边栏入口

在 [`IconSidebar.tsx`](../../src/components/common/IconSidebar.tsx) 的「知识空间」入口**正下方**插入「设计空间」入口（lucide 图标，建议 `Palette` / `Shapes` / `PenTool`，`t("design.title")`，`view === "design"` 高亮，`onClick={onOpenDesign}`）。同步 `App.tsx` 的 `view` 联合、`lazy` import、渲染分支、`onOpenDesign` prop。

---

## 10. 导出与产物库

### 10.1 导出格式

| 格式 | 做法 | 说明 |
| --- | --- | --- |
| **HTML** | 直接产出 `index.html`（已自包含内联） | 单文件，零依赖 |
| **PNG** | inspector bridge `html2canvas`（vendored 本地）截图，或 owner 端 headless 光栅化 | 缩略图同源 |
| **PDF** | `deck` → 每页一 PDF page；`web/poster/document` → 整页 | 走 webview print-to-pdf / 零依赖 PDF 写出器；保真优先，见 roadmap Phase 5 |
| **PPTX** | 确定性 pptx 写出器（freeform 页型），每 `slide`/`poster` 一页 | 保真优先（渐变/字体替换是行业公认痛点，重点投入） |

`exports/` 目录**必须 gitignore**（restore 会清）；HTTP 导出路由需 `DefaultBodyLimit` 放开。

### 10.2 产物库

统一缩略图墙（跨项目/项目内）+ 版本对比（并排 iframe / 缩略图 diff）+ 批量导出 + 分享入口。owner 平面读 `list_artifacts` / `get_artifact`，取消/删除复用统一入口。

---

## 11. 质量评审门与设计方向选择器

### 11.1 5 维质量门

`design(action="critique", artifact_id | html)` 走 [side_query](side-query.md)（复用主 system prompt 前缀命中 cache，成本低）对产物做 5 维评审：**品牌契合 / 可访问性(a11y) / 视觉层次 / 可用性 / 性能**，返回每维评分 + 具体可执行修复 + 总分。可配 `auto_critique` 在 finalize 前自动跑（反 AI-slop：占位内容 / 变体雷同 / 对比度不足）。总分落 `critique_score`（版本级）。

### 11.2 设计方向选择器

brief 缺设计系统时，`design(action="propose_directions", brief)` 返回 N（默认 4）个方向选项（每个是一份 mini 设计系统预览：色板 + 字体 + 一个样例组件）。前端渲染为可选卡片，用户选定即作为该产物/项目的设计系统；也可"从截图/URL 导入"走 D2 反向提取。

---

## 12. 与现有子系统的契约

- **[知识空间](knowledge-base.md)（D4）**：`save_to_knowledge` 生成 KB 笔记内嵌产物预览链接 + 元数据 → 设计产物进第二大脑可检索；读取即 untrusted 信封约束不变。
- **[项目](project.md)（D4）**：设计项目可绑定 `ha_project_id` 共享工作目录；`extract_system from=codebase` 读的是绑定项目/会话工作目录内文件（走 `WorkspaceScope` 作用域闭合）。
- **[系统提示词](prompt-system.md)（D4）**：会话可附着一个设计系统，`design` prompt 段以名称 + 气质摘要注入（预算受控、静态 prefix cache 友好），像[记忆](memory.md)/知识那样约束生成；incognito 零注入。
- **[图片生成](image-generation.md)**：`image` kind 复用现有 7 Provider trait 抽象，不重造。
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
| 版本/恢复 | `design_artifact_versions/restore_cmd` | `GET/POST …/artifacts/{aid}/versions` | 同名 |
| 可视化回写 | `design_patch_element_cmd` | `POST …/artifacts/{aid}/patch` | 同名 |
| 设计系统 CRUD | `list/get/save/delete_design_system_cmd` | `…/api/design/systems[/{id}]` | 同名 |
| 反向提取 | `design_extract_system_cmd` | `POST /api/design/systems/extract` | 同名 |
| 导出 | `design_export_cmd` | `POST …/artifacts/{aid}/export` | 同名 |
| 质量门 | `design_critique_cmd` | `POST …/artifacts/{aid}/critique` | 同名 |
| 缩略图 | `design_save/get_thumbnail_cmd` | `…/artifacts/{aid}/thumbnail` | 同名 |
| snapshot 回传 | `design_submit_snapshot_cmd` | `POST /api/design/snapshot/{requestId}` | 同名 |
| 静态托管 | （Tauri `asset://` 直读） | `GET /api/design/projects/{pid}/artifacts/{aid}/{*rest}` | iframe 直连 |
| 配置读写 | `get/save_design_config_cmd` | `GET/PUT /api/config/design` | 同名 |

---

## 16. 文件清单（注册触点）

新增平级子系统的注册触点（坐标来自现网 knowledge 子系统对照）：

### 后端

| 文件 | 角色 |
| --- | --- |
| `crates/ha-core/src/design/{mod,service,db,renderer,system,patch,critique,export,recipe}.rs` | 核心：注册表 + 业务 + 渲染 + token 编译 + oid 回写 + 质量门 + 导出 + 模板 |
| `crates/ha-core/src/tools/design/mod.rs` | `design` agent 工具（多 action 路由） |
| `crates/ha-core/src/lib.rs` | `pub mod design;`（挨着 `pub mod knowledge;`） |
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
| `src/components/design/DesignInspector.tsx` + `inspector/` | 属性检视器（分区控件，纯函数 class/style ⇄ 属性模型） |
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
