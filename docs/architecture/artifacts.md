# Artifacts 本地优先产物平台

> 返回 [文档索引](../README.md)
>
> 更新时间：2026-07-14

Artifacts 是 Canvas 之上的持久化控制面。Canvas 继续负责 HTML 运行与右侧预览；Artifacts 负责身份、不可变版本、来源、证据、验证、导出、归档和 Gallery。旧 Canvas ID 原样成为 Artifact ID，不迁移 `canvas.db` 或历史项目目录。

## 边界与不变量

- 业务逻辑全部在 `ha-core::artifacts`；Tauri 与 HTTP 只做薄壳。
- 旧 `canvas` 工具与 `/api/canvas/*` 保持兼容。旧记录惰性登记为 `kind=custom`、`privacy=local_private`、`producer=legacy_canvas`。
- Artifact 版本不可变；update 必须携带 `expected_version`，restore 生成新版本。
- 模型工具只有 create/update/show/list/versions/restore/verify。导出、复核、归档和删除只在 owner 平面。
- Artifact 只引用 Domain Evidence ID；`domain_evidence_items` 仍是证据真相源。
- `incognito` 请求禁止进入持久化入口。当前版本选择 fail-closed，不提供伪装成内存持久化的 Gallery 记录。

## 存储

现有 `canvas_projects` / `canvas_versions` 保持不动，并加法新增：

- `artifact_records`：当前控制面元数据、隐私、能力清单和验证摘要；
- `artifact_version_meta`：版本 parent、payload、hash、producer、source/evidence 摘要；
- `artifact_exports`：导出 receipt、hash、验证、内部受管路径和过期时间；
- `artifact_blobs` / `artifact_version_blobs`：SHA-256 内容寻址 payload 与版本引用。

项目当前预览仍落 `~/.hope-agent/canvas/projects/{id}/`。导入会复制并规范化到 managed store，不保留活动源链接。所有受管文件写入走 `platform::write_atomic`；版本删除后按引用做 blob GC。

## AnalysisArtifactV1

`hope.analysis-artifact.v1` 是 Hope 原生 Data Analytics 交换契约，包含问题、受众、决策、状态、指标口径、时间范围、bounded datasets、findings、recommendations、caveats、blocks、charts、tables、fallbacks、canonical sources、data quality 和 claim validation。

Core 拒绝：

- 未知 schema、空问题或非法状态；
- 无稳定 ID/hash 的来源；
- 无 rowCount/rows 的 dataset，或内嵌超过 5000 行；
- chart 缺 dataset/source/fallback 或引用不存在；
- `ready` 中存在 failed blocking quality check。

内置 `skills/ha-data-analytics/` 按 context → sources → quality → analysis → visualization → report → validation → register 执行，并提供依赖无关 validator 与 CSV/XLSX golden fixtures。Artifact 创建会把显式结构映射为 `source_cited`、`data_quality_checked`、`claim_checked`、`artifact_created`。

## API 与 Transport

| Owner 动作 | HTTP | Tauri |
| --- | --- | --- |
| list/get/delete | `GET/DELETE /api/artifacts...` | `list/get/delete_artifact` |
| import/update | `POST /api/artifacts/import` | `import_artifact` |
| versions/restore | `GET .../versions`, `POST .../restore` | `list_artifact_versions`, `restore_artifact` |
| verify | `POST .../verify` | `verify_artifact` |
| export review | `POST .../export-review` | `review_artifact_export` |
| export/download | `POST .../exports`, streaming `GET /api/artifact-exports/{id}/download` | `export_artifact` + native save dialog |
| archive | `POST .../archive` | `archive_artifact` |

HTTP 与 Tauri 前端必须只经 `Transport` 抽象访问。大文件使用受管文件复制或 HTTP streaming，不经 JSON/base64 IPC。

## 离线渲染与导出

- Analysis/Markdown 由确定性 Rust renderer 生成语义 HTML；无 CDN。
- Freeform HTML 导入时强制叠加离线 CSP，拒绝 iframe/object/embed/form，并在 capability manifest 标记脚本/可执行内容。
- verifier 检查 HTML、CSP、远程资源/API、禁止元素和语义 fallback。
- HTML、Markdown、ZIP 都由同一 canonical version 生成。ZIP manifest 记录每个成员的 MIME、大小和 SHA-256，不默认包含聊天、附件原件或连接器内容。
- PDF 使用 app-owned managed Chromium 的 `Page.printToPDF`；校验 PDF magic、页数、文本可提取性和 hash。runtime 不可用时保存 failed receipt，HTML/ZIP/Markdown 不受影响。

Freeform HTML 即使通过“无已知远程依赖”验证仍属于可执行内容；接收者直接打开不等同于 Hope iframe 沙盒。

## Export Guard

`shareable_snapshot`、`sensitive`、private/connector/sensitive source 或 `redistributable=false` 会调用既有 `evaluate_domain_artifact_export_guard`。Gallery 的 owner-side 复核记录目标受众、`exportReview`、`exportReady`、`redactionChecked` 和 `artifact_reviewed` evidence。Guard 未通过时 Core 导出入口 fail closed，前端绕过无效。

Publisher 不属于 export 权限。LAN、企业存储、WebDAV、Drive 或 Sites 后续必须各自提供 strict approval、凭据、保留期、撤销和审计适配器。

## 前端

顶层 `ArtifactsView` 提供分页、类型/状态筛选、统一 `ArtifactViewer`、验证、版本历史、restore、复核、导出、归档和删除。`CanvasPanel` 复用同一 Viewer。首版不提供源码编辑器或富文本编辑器；正文通过 Agent 的 optimistic-concurrency update 维护。

## 后续阶段

- 补齐 assets 解析/复制和多资源 blob manifest；当前 blob store覆盖 canonical payload。
- `SlidePlanV1`、editable PPTX 与布局/视觉 QA。
- 真正的 incognito 内存 Viewer；在此之前持久化入口保持拒绝。
- 独立 Publisher adapters；不得复用普通导出授权。
