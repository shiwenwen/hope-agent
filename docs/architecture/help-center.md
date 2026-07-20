# 内置用户手册（帮助中心）

> 对外名「使用手册」。单一真相源是仓库的 [`docs/user-guide/`](../user-guide/README.md)（中文根目录 + `en/`，13 章 + README），本文描述它如何被编进应用、在 GUI 与 agent 两条路径上被消费。

## 总览

```
docs/user-guide/{,en/}*.md          ← 单一真相源（改文档 = 改手册）
        │ rust-embed（编译期内嵌进 ha-core）
        ▼
crates/ha-core/src/manual/          ← 解析 + 搜索 + 镜像
        ├─ GUI 路径：get_manual_bundle / search_manual（Tauri + HTTP）
        │            → HelpWindow（独立窗口 / Web 新标签页）
        └─ Agent 路径：镜像 <data-dir>/manual/{zh,en}/NN.md
                     → ha-manual skill → read/grep
```

与 `skills/embedded.rs`（bundled skills）、`browser/extension/embedded.rs`（Chrome 扩展）同属「rust-embed 单一来源 + 全部署形态统一携带」体系：**禁止再往构建产物单独拷贝手册**（Tauri `bundle.resources` / Dockerfile runtime COPY 均不需要）。

## 模块边界（`crates/ha-core/src/manual/`）

| 文件 | 职责 |
| --- | --- |
| `embed.rs` | `#[derive(RustEmbed)] #[folder = "../../docs/user-guide"]`；debug 读盘（改 md 免重编）、release 内嵌。`build.rs` 的 `rerun-if-changed=../../docs/user-guide` 保证 warm rebuild 不带旧文件集 |
| `model.rs` | 解析：语言按 `en/` 前缀、章节号按文件名 `NN` 前缀（README=0）——**绝不用中文文件名字面量查内嵌 key**（macOS NFD / Linux NFC 归一化差异会静默 miss）；heading 扫描（fence-aware）+ GitHub 风格 slug |
| `search.rs` | 逐行 + 空白切词 + 逐词 Unicode 子串 AND（覆盖中英混合，无需分词器）；字符偏移（非字节）；snippet 用 STX/ETX（`\u{2}`/`\u{3}`）标记命中——与会话搜索 `renderHighlightedSnippet` 同一契约 |
| `unpack.rs` | 镜像到 `<data-dir>/manual/`：逐文件字节 diff + prune + 兄弟位指纹 marker（`.manual-synced`，存内嵌**源集** BLAKE3 指纹——二进制升级但 docs 未变则短路，变了自动重镜像）。**每次调用都做廉价校验**（源指纹进程内缓存 + 每个期望文件一次 stat，**刻意不缓存"已镜像"路径**），全量或部分删除（单章 / 整个语言目录）在下次触发即重建——守住「safe to delete — rebuilt on next use」承诺；昂贵的 derive+写盘只在校验 miss 时跑。进程内互斥；跨进程混版本共存是已注释的已知限制 |
| `mod.rs` | 类型 + 公开 API（`get_bundle` / `search` / `manual_language_for_locale` / `ensure_local_manual`）+ 命令层入口 |

## 关键契约

### Slug 一致性（三方逐字节一致）

Rust 端 slug（`model.rs::github_slug`）、手册正文里的 intra-doc `#anchor` 链接、前端渲染注入的 heading `id`（[`manualSlug.ts`](../../src/lib/manual/manualSlug.ts)）三者必须一致，否则跳转与搜索定位静默失效。守卫：

- Rust `every_intra_doc_anchor_resolves_to_a_computed_slug`——对**全语料**（两语言全部章节的全部锚点链接）断言可解析；
- Rust / TS 各有一份**相同 ground-truth 对**的单测（取自真实文档锚点），双端锁死 CJK 边角（标点塌缩、双连字符、括号删除）。

算法：trim → 空格转 `-`、保留 `-`/`_`、Unicode 字母数字转小写保留、其余丢弃；重复 slug 追加 `-1`、`-2`（同 github-slugger，仅 Rust 侧实现全局去重）。Streamdown 默认 rehype（raw/sanitize/harden）**不注入 heading id**，由 [`HelpMarkdown.tsx`](../../src/components/help/HelpMarkdown.tsx) 的 rehype plugin 注入——**id 直接取 bundle 携带的权威 `headings[].slug`**（按标题文本匹配；`manualSlug.ts` 只作 bundle 未覆盖文本的后备），前端不做重复后缀去重（Streamdown 按 block 跑 rehype 无法全局计数；手册 `N.M` 编号保证章内标题唯一，Rust 语料测试守护实际存在的锚点）。

### 镜像布局（ASCII basename）

磁盘镜像写 `manual/{zh,en}/NN.md` + `index.md`（README），**CJK 文件名绝不落盘**——规避 Windows 非 ASCII 与 NFD/NFC 两类跨平台坑。镜像副本内的跨章链接被确定性重写为 ASCII 名（`NN-xxx.md` → `NN.md`、`README.md` → `index.md`、两条语言切换链接 → `../<lang>/index.md`），保持可跟随。marker 放 `manual/` **兄弟位**（不被 prune 扫掉）；目录可安全删除，下次使用重建。

### 镜像触发点（三处，全部幂等）

1. 启动：`app_init.rs` 的 `start_background_tasks` / `start_minimal_background_tasks`（ACP）primary-only 块，`spawn_blocking` 不占 runtime worker；
2. `ha-manual` skill 激活时——特判放在 [`tools/skill/inline.rs`](../../crates/ha-core/src/tools/skill/inline.rs) 的 `execute`（**两条激活路径的共同咽喉**：模型 `skill({name})` 工具调用与用户 `/manual` 斜杠命令都经它，启动镜像失败在任一入口重试都生效）；
3. `get_manual_bundle` 命令（HelpWindow 打开是 agent 路径的自然就绪点）。

指纹命中即约 30 次 stat 的廉价校验短路。失败一律 `app_warn!`（category `manual`）不致命——GUI 读内嵌不依赖磁盘。

## GUI 路径

- **命令**（Tauri + HTTP 双实现，见 [api-reference.md](api-reference.md)）：`get_manual_bundle(lang?)` 一次返回全部章节 + headings（导航/大纲/Cmd+F/跨章跳转全在前端本地完成）；`search_manual(lang?, query)` 保持 CJK 排序与 snippet 逻辑在 Rust 单一来源。`lang` 缺省用 `i18n::current_ui_locale()`；`manual_language_for_locale`：zh / zh-TW → zh 手册，其余 → en（UI chrome 仍是 12 语）。
- **独立窗口**：`?window=help` 走 [`main.tsx`](../../src/main.tsx) 既有单入口分流（动态 import，不进主 chunk）。桌面 `help-window` label（登记于 `capabilities/default.json` 的 `windows`），get-or-create + `help:navigate` 事件 re-target；Web 模式降级为同源新标签页（token 经 localStorage 自动携带）。
- **链接改写 + 拦截（harden 红线）**：Streamdown 默认 rehype 链里的 rehype-harden（无 `defaultOrigin`）会把**裸相对 href**（`02-模型与Provider.md`）替换成不可点的 Blocked-URL span——因此 [`helpLinks.ts`](../../src/lib/manual/helpLinks.ts) 的 `rewriteManualBody` 在**渲染前**把五类链接形态（anchor / chapter / language-switch / external / none）改写成能过 harden 的目标：章节 → `#ch:N[:anchor]` fragment、语言切换 → `#lang-switch`、越出手册的相对链接按语言深度解析成绝对 GitHub URL、未识别形态留给 harden 中和（本就**不导航**）。点击在容器 capture 阶段经 `resolveRenderedHref` 还原并路由，**不改共享 `MarkdownLink` 默认行为**。
- **入口**：侧边栏帮助图标（设置齿轮上方）、AboutPanel 按钮、macOS 原生 Help 菜单 + 三平台托盘项（Rust 侧 emit `open-help`，App 监听后 `openHelpWindow()`）、设置页高频面板 header 的「?」深链（`SettingsView` 的 `HELP_CHAPTER_BY_SECTION`，只链章节号——锚点是语言相关的，章节号是跨语言 join key）。
- **问 AI**：HelpWindow 把章节引用/选中文本经 `help:ask-ai`（桌面 Tauri 全局事件 / Web BroadcastChannel）送到 App → 切到聊天视图 → 经模块级队列（`askAi.ts`，免 mount 竞态）作为 **message-quote chip** staged 进 composer——复用既有 `PendingMessageQuote` 机制，不动 ChatInput。
- **章节内 Cmd+F**：TreeWalker 定位 + CSS Custom Highlight API 高亮（`::highlight(help-find)`，不可用时降级为仅计数 + 滚动，不改 React DOM）。

## Agent 路径

[`skills/ha-manual/SKILL.md`](../../skills/ha-manual/SKILL.md)：单文件 skill（无 fork、只读），内联章节路由表（按 `NN.md` 引导，通常一次 `read` 即中）。**路径动态解析** `${HA_DATA_DIR:-$HOME/.hope-agent}/manual/`（Docker 的 data-dir 是 `/data`，写死 `~` 会 ls 到空）。skill 本体随 #506 的 bundled-skills 内嵌机制在全部部署形态被目录发现。**单一来源纪律：SKILL.md 只放路由表，绝不复制手册正文**。守卫：`ha_manual_skill_routing_table_matches_chapters`（cargo test）断言路由表引用集合与真实章节一致。

## 双语对齐守卫

[`scripts/check-docs-parity.mjs`](../../scripts/check-docs-parity.mjs)（`pnpm check:docs-parity`，lint.yml step）：章节号集合 1:1、每章 H2/H3 计数一致、两 README 章节链接可解析。中英必须同 PR 更新（AGENTS.md 文档维护表）。

## 测试地图

- **cargo（`manual::` 16 项）**：embed 非空硬门禁（`iter()` 计数，防 Docker 缺 COPY 静默空手册）、章节解析、slug 语料契约、CJK 搜索、镜像幂等/指纹短路/重镜像、链接重写形状、skill 路由表防漂移、语言映射。
- **vitest**：`manualSlug.test.ts`（与 Rust 共享 ground-truth 对）、`helpLinks.test.ts`（五类链接形态穷举 + 越界不导航）。
- **手测面**：开窗/聚焦/关闭、菜单与托盘入口、Web 新标签页、搜索高亮定位、Cmd+F、大纲跳转、设置页深链、问 AI、语言切换、Docker 内 agent 激活 ha-manual。
