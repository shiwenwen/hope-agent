# 设计空间（Design Space）迭代计划

> 返回 [文档索引](../README.md) · 架构见 [`design-space.md`](design-space.md)
>
> 本文是设计空间从零到产品级的**迭代计划真相源**：分期、每期工作流、验收标准、能力复刻对照矩阵、决策账本。目标（用户拍板）：**完整复刻 agent 原生设计工作空间的全部能力，并在四个方向上做到"好用 + 完美"，达到产品级**。

## 实施状态（as-built）

**已交付（产品级，全套 CI 门禁绿 + 真磁盘端到端冒烟通过）：**

- ✅ 架构文档 + 本迭代计划
- ✅ P1 工作空间（首页项目墙 + 工作室产物库 + 稳定单产物 iframe 预览，无画布）+ 项目/产物 CRUD
- ✅ P2 设计系统（6 套原创品牌契约 + token 编译注入 + 系统 CRUD + 项目品牌切换）
- ✅ P3 渲染管线（7 种 HTML 形态自包含产物 + deck 翻页器 + token 注入）+ agent `design` 工具（18 action）+ 模板目录 + 版本更新/恢复
- ✅ P4 可视化直接微调（D1）：oid 确定性字节回写 + inspector bridge + 分区检视器 + stale-write 守卫
- ✅ P5 一键导出 **HTML / PNG / PDF / PPTX**（HTML 后端干净自包含；PNG/PDF 前端 html2canvas+jsPDF 客户端栅格化，非打断、两模式通用；PPTX 前端栅格化整页图 + 后端 `zip`+OOXML 组装）
- ✅ P6 5 维质量评审门（critique）+ 设计方向选择器（propose_directions）
- ✅ **D2 反向提取四通道**：文本描述 · 本地代码库 · **URL（抓原始 HTML）** · **截图（视觉模型单发，`design/vision.rs` 隔离，零改主对话）**
- ✅ **D1 image 形态**：接线现有 `image_generate` Provider 栈（`design/image.rs`，data-uri 内嵌保自包含），owner/agent 共用 `create_artifact_generating`
- ✅ **motion 形态**：自包含 CSS/JS 动画（1280×720，一等交付；HTML/PNG/PDF 导出）
- ✅ **owner 富 UI**：版本历史面板（列版本 + 恢复）· 反向提取对话框（四通道）· 设计方向选择卡片（色板预览 + 一键采用）· 导出格式菜单 · image 生成入口
- ✅ D4 知识空间联动（save_to_knowledge）
- ✅ 设置三件套 · 文档同步（AGENTS.md/api-reference/诊断索引/CHANGELOG）· **12 语完整本地化**（design 命名空间全 12 语真人级翻译，`sync-i18n --check` 零缺失）

**明确的边界（非缺口，硬性依赖使然）：**

- **视频 MP4 文件编码（HTML→MP4）**——需打包 headless Chrome + FFmpeg 重原生依赖，与本产品"轻量、零重依赖"红线冲突（DQ9）。`motion` 形态已交付动画能力（创建/预览/迭代/HTML 导出），MP4 二进制编码留给用户环境已有的 ffmpeg（不捆绑，避免脆弱半成品）。截图提取当前覆盖 Anthropic / OpenAI-Chat 两大 vision 格式（占绝大多数配置），Responses/Codex provider 给出明确"切换到 vision 模型"提示而非报错。

## 0. 背景与目标

- **起点**：既有 `feat/atelier` 分支（"设计工坊"）验收不通过——**画布交互卡顿不稳、渲染重且易白屏、可视化微调不好用**。用户要求推倒重做。
- **参考系**：agent 原生设计工作空间品类（自包含 HTML 产物 + 沙盒 iframe 预览 + 设计系统 + 技能/模板 + 多形态导出 + 质量门）。
- **本版分水岭**：轻量自包含 HTML（拒浏览器内编译）+ 产物墙（拒无限画布）+ 纯 HTML 确定性回写（做扎实可视化微调）。详见 [`design-space.md` §1.2](design-space.md#12-设计原则每一条都直接回应旧版设计工坊的失败点)。
- **完成定义**：能力复刻矩阵（§4）全绿 + 四大差异化（D1–D4）产品级 + 架构文档与本计划完整 + 12 语 i18n + 设置三件套 + 测试绿。

## 1. 分期总览

| Phase | 主题 | 交付 | 依赖 |
| --- | --- | --- | --- |
| **P0** | 文档与骨架 | 架构文档 + 本计划 + worktree + 后端模块骨架 + 侧边栏入口点亮（空视图） | — |
| **P1** | 基础设施与 CRUD | `design.db` + paths + service + Tauri/HTTP/transport 三层贯通 + 项目/产物 CRUD + 首页/工作室基础 UI | P0 |
| **P2** | 设计系统 | `SYSTEM.md` 解析 + token 编译 + 内置系统 + 目录 UI + 编辑器 + 反向提取（截图/URL/代码库，D2） | P1 |
| **P3** | 渲染与生成 | 自包含 HTML 渲染管线 + oid 标注 + 8 种 kind 骨架 + 模板目录 + `design` agent 工具 + 生成状态机 | P1 |
| **P4** | 预览与可视化微调 | 稳定单产物预览 + inspector bridge + 检视器分区控件 + oid 确定性回写（D1）+ 版本历史 + 批注 | P3 |
| **P5** | 导出与产物库 | HTML/PDF/PPTX/PNG 导出 + 产物库缩略图墙 + 版本对比 + 分享（D3） | P3 |
| **P6** | 质量与联动 | 5 维质量门 + 反 AI-slop 自查 + 方向选择器 + 知识空间/项目联动（D4）+ 设置三件套 | P2,P4 |
| **P7** | 收尾与硬化 | 12 语 i18n 齐 + 对抗 review 全修 + 文档同步 + 桌面 App 内驱动验证 + 性能基准 | 全部 |

每个 Phase 完成即在本仓库 `feat/design-space` 分支提交（commit 标题含 🦭），跨 Phase 不 merge main（仅 cherry-pick，若需要）。

## 2. 各期工作流与验收标准

### P0 · 文档与骨架
**工作流**：① 写 `design-space.md`（架构）② 写本计划 ③ 登记 `docs/README.md` 索引 ④ 后端建 `design/` 模块空骨架（编译过）⑤ 前端点亮侧边栏「设计空间」入口 + 空 `DesignView`。
**验收**：`cargo check -p ha-core` 过；`pnpm typecheck` 过；侧边栏「知识空间」下方出现「设计空间」入口，点击进入空视图不崩。

### P1 · 基础设施与 CRUD
**工作流**：`design.db` 建表 + `paths` + `service` owner 入口（项目/产物 CRUD、列表、reindex）+ Tauri 命令 + HTTP 路由 + `transport-http` COMMAND_MAP + 前端 `DesignHome`（最近项目墙）/`DesignStudio`（产物库空态）+ 项目/产物增删改查跑通。
**验收**：能在 UI 新建项目、新建空产物（占位 index.html）、删除、切换；Tauri 与 HTTP 两种模式均可用；删除项目级联清目录 + DB。

### P2 · 设计系统（含 D2）
**工作流**：`SYSTEM.md` 9 段解析 + `compile_tokens` + 8 套内置系统（`design-assets/systems/`）+ 系统目录 UI + 可视化 token 编辑器 + `extract_system`（image/url/codebase 三源，走多模态/SSRF/WorkspaceScope）。
**验收**：能浏览/新建/编辑/删除设计系统；截图/URL/本地代码库三种反向提取各出一份合理 `SYSTEM.md` + tokens；token 改动重编译 CSS 变量；incognito 零注入。

### P3 · 渲染与生成
**工作流**：`renderer::build_artifact_html`（8 kind 骨架 + token 注入 + oid 标注 + bridge 注入 + 零网络）+ `RECIPE.md` 模板目录 + `design` 工具（plan/create/update/list/get/snapshot 等 action）+ 生成状态机（planned→generating→ready/failed）+ 事件链。
**验收**：模型经 `design` 工具能产出各 kind 产物并在工作室即时渲染（无白屏、启动 < 300ms）；批量生成出骨架卡逐个点亮；snapshot 回多模态自反馈；套用不同设计系统换皮生效。

### P4 · 预览与可视化微调（D1，重点）
**工作流**：稳定单产物 iframe 预览（纯 CSS 缩放，无画布）+ inspector bridge（select/hover/文本编辑/snapshot）+ `DesignInspector` 分区控件（布局/间距/尺寸/排版/填充/描边，class/style ⇄ 属性纯函数模型）+ `patch_element` oid 确定性回写（`expected` 守卫 + undo/redo）+ 版本历史 UI + 批注钉。
**验收**：选中元素改文案/配色/间距**即时预览 + 准确回写源码**，改 100 次不出错、不改坏、无卡顿；stale-write 正确拒绝；撤销/重做正确；版本可回溯；**这是对旧版三痛点的正面验收——必须明显好用**。

### P5 · 导出与产物库（D3）
**工作流**：HTML（直出）+ PNG（bridge 截图/owner 光栅化）+ PDF（webview print / 零依赖写出器，deck 分页）+ PPTX（确定性 freeform 写出器 + 质量闸）+ 产物库缩略图墙 + 版本并排对比 + 批量导出 + 分享入口。`exports/` gitignore + HTTP body limit。
**验收**：四格式导出各得可用文件，PDF/PPTX 保真达标（渐变/字体不明显丢失）；产物库跨项目浏览、版本对比、批量导出。

### P6 · 质量与联动（D4）
**工作流**：`critique` 5 维（side_query）+ `auto_critique` 门 + 反 AI-slop 自查 + `propose_directions` 方向选择器 + `save_to_knowledge`（KB 笔记沉淀）+ 设计系统 system-prompt 注入 + 项目绑定 + 设置三件套。
**验收**：质量门给出可执行评分 + 修复；方向选择器出 4 个可选方向卡；产物一键沉淀进知识空间可检索；会话附着设计系统后生成受其约束；GUI/技能/文档三件套零偏差。

### P7 · 收尾与硬化
**工作流**：12 语 i18n 补齐（`node scripts/sync-i18n.mjs --check`）+ 六维对抗 review 全修 + 架构文档/api-reference/ha-self-diagnosis 索引同步 + CHANGELOG + 桌面 App 内逐层驱动验证（host 装载→渲染→交互）+ 画布/回写热路径性能基准。
**验收**：`pnpm test` / `cargo test -p ha-core -p ha-server` 绿；i18n --check 无缺；桌面 App 内全链路可用；性能达标（预览渲染/回写 commit 无感）。

## 3. 关键工程约束（红线速查）

- **零 Tauri 依赖**：核心全进 `ha-core`；`src-tauri`/`ha-server` 薄壳。
- **Transport 双实现**：每个 invoke 同时 Tauri + HTTP。
- **写盘原子**：`platform::write_atomic`，禁 `fs::write` 回退。
- **沙箱三闸**：id 白名单 + safe rest path + `contained_canonical`。
- **沙箱消息不可信**：iframe→磁盘写走父窗确认 + 数值净化 + 白名单令牌 + `expected` stale-write。
- **SSRF**：`extract_system from=url` 必过 `security::ssrf::check_url`。
- **外部只读**：后台自主维护绝不写外部工程；提取默认落 managed。
- **incognito**：零注入、不沉淀、不进全局索引。
- **配置**：`mutate_config` 写、`cached_config` 读，设置三件套。
- **无外部项目名**：代码/注释/commit/文档/UI/i18n 均不出现任何外部参考实现名。
- **文件即真相源**：`design.db` 可重建；产物/系统正文在磁盘。

## 4. 能力复刻对照矩阵（参考品类 → 本版）

> 目标是**完全复刻**参考品类的能力，并标注本版超越点。「本版位置」列指向落地 Phase。

| 参考品类能力 | 本版对应 | 超越点 | Phase |
| --- | --- | --- | --- |
| 原型生成（web/mobile/desktop） | `web`/`mobile` kind 自包含 HTML | 可视化微调回写 | P3,P4 |
| 演示文稿（deck，多模板多主题） | `deck` kind + 自带翻页器 + 模板 | 就地编辑 slide 元素 | P3,P4 |
| 仪表盘 / live artifact | `dashboard` kind | 与本地数据/知识联动 | P3,P6 |
| 图像生成 | `image` kind 复用现有 7 Provider | —— | P3 |
| 文档 / one-pager | `document` kind | —— | P3 |
| 海报 / 社媒图 | `poster` kind 尺寸预设 | —— | P3 |
| 邮件营销 | `email` kind table 回退 | —— | P3 |
| 沙盒 iframe 预览 | 稳定单产物 iframe（无画布） | 不白屏、启动快、无卡顿 | P3,P4 |
| 设计系统（DESIGN 契约 9 段） | `SYSTEM.md` 9 段 + token 编译 | 原创设计语言、非品牌克隆 | P2 |
| 内置设计系统库（大量品牌） | 8 套原创原型化系统 + 用户自建/提取 | 规避商标 + 本地提取护城河 | P2 |
| 从截图/URL/codebase 提取设计系统 | `extract_system`（image/url/codebase） | **读本地工程**（云端做不到） | P2 |
| 技能/模板目录（按 mode/scenario） | `RECIPE.md` 模板目录 | —— | P3 |
| 5 维自评质量门 | `critique` 5 维 side_query | 降本 cache 复用 | P6 |
| 设计方向选择器 | `propose_directions` 4 方向卡 | —— | P6 |
| 导出 HTML/PDF/PPTX/PNG/ZIP | HTML/PDF/PPTX/PNG 导出 | PPTX 保真重点投入 | P5 |
| 可视化直接编辑（文本/间距/颜色） | inspector bridge + oid 回写 | **纯 HTML 确定性映射**（做扎实） | P4 |
| 批注 / inline comment | 批注钉回灌对话 | —— | P4 |
| 产物版本 / 变体 | 快照版本 + 多变体并排 | —— | P4 |
| 沉淀 / 分享 | `save_to_knowledge` + 分享入口 | 进第二大脑可检索 | P5,P6 |
| MCP / 多 agent 集成 | Hope Agent 自身即 agent 平台 | 原生 | 既有 |

**不做/降级**（明确边界，避免范围蔓延）：

- **视频 / 动效（HTML→MP4）**：需 headless Chrome + FFmpeg 重型原生依赖，桌面 App 内成本高、脆弱。**首轮不做**，列为 P7 之后可选迭代（届时评估 vendored ffmpeg 成本）。矩阵完成不以此为门槛。
- **无限画布多画板同屏拖拽**：**刻意不做**——正是旧版卡顿之源。以产物库缩略图墙 + 单产物聚焦替代（体验更稳）。

## 5. 决策账本（Open Questions 与拍板）

| # | 决策 | 结论 | 依据 |
| --- | --- | --- | --- |
| DQ1 | 产品名 | **设计空间**（代码 `design`） | 用户拍板，与「知识空间」对仗 |
| DQ2 | 是否复用/改进 atelier | **推倒重做，独立子系统** | 用户："那个做的不好，才叫你重新做" |
| DQ3 | 渲染路线 | **轻量自包含 HTML + iframe 直载**，拒浏览器内编译 | 旧版痛点：渲染重/白屏 |
| DQ4 | 工作区形态 | **产物库 + 单产物聚焦**，拒无限画布 | 旧版痛点：画布卡/不稳 |
| DQ5 | 可视化微调 | **纯 HTML + oid 确定性字节回写** | 旧版痛点：微调不好用；纯 HTML 映射无损 |
| DQ6 | 内置设计系统 | **原创原型化，非品牌克隆** | 规避商标 + 消除抄袭痕迹 |
| DQ7 | 差异化取舍 | **D1–D4 全做且做到好用+完美** | 用户拍板 |
| DQ8 | 真相源 | **磁盘正文 + DB 可重建索引** | 对齐知识空间 D9 |
| DQ9 | 视频/动效 | **首轮不做**（重原生依赖），列后续可选 | 范围控制 |
| DQ10 | 独立表 vs 复用 canvas | **独立 `design.db`/工具/视图**，不碰 canvas | 用户："不要管旧的那条 canvas" |

## 6. 里程碑与提交节奏

- 每个 Phase 一组语义提交，标题含 🦭 + conventional-commit 前缀。
- Phase 收尾若跨多模块，主动跑 `cargo clippy`/`cargo test`/`pnpm typecheck`/`pnpm test` 收尾（改动较大时）。
- P4（可视化微调）与 P5（导出）各自做一轮对抗 review（安全 + 回写正确性 + 保真）。
- 全部完成后：架构文档终稿 + 本计划标注 as-built + 归档研究/计划原始稿。
