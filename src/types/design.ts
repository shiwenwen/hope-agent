/**
 * 设计空间（Design Space）前端类型。
 *
 * 与 `crates/ha-core/src/design/` 的 serde camelCase 输出对齐。
 */

/** 产物形态。 */
export type ArtifactKind =
  | "web"
  | "mobile"
  | "deck"
  | "dashboard"
  | "poster"
  | "document"
  | "email"
  | "image"
  | "motion"
  | "audio"
  | "component";

/** 产物生成状态。 */
export type ArtifactStatus =
  | "planned"
  | "generating"
  | "ready"
  | "failed"
  | "needs_review";

/** 反-slop 自查命中详情，存于 `DesignArtifact.metadata` 的 `selfCheck` 键。 */
export interface SelfCheckFlag {
  flag: string;
  detail: string;
}

/** 从产物 metadata（JSON 字符串）解析自查命中；无 / 解析失败 → null。 */
export function parseSelfCheck(metadata?: string | null): SelfCheckFlag | null {
  if (!metadata) return null;
  try {
    const obj = JSON.parse(metadata) as { selfCheck?: SelfCheckFlag };
    return obj?.selfCheck?.detail ? obj.selfCheck : null;
  } catch {
    return null;
  }
}

/** 从产物 metadata 解析血缘来源（派生自哪个产物）；无 / 解析失败 → null。 */
export function parseDerivedFrom(
  metadata?: string | null,
): { id: string; title: string } | null {
  if (!metadata) return null;
  try {
    const obj = JSON.parse(metadata) as { derivedFrom?: { id?: unknown; title?: unknown } };
    const d = obj?.derivedFrom;
    if (d && typeof d.id === "string" && typeof d.title === "string") {
      return { id: d.id, title: d.title };
    }
    return null;
  } catch {
    return null;
  }
}

/** 从产物 metadata 解析 deck 演讲者备注（按 slide 顺序）；无 / 解析失败 → []。 */
export function parsePresenterNotes(metadata?: string | null): string[] {
  if (!metadata) return [];
  try {
    const obj = JSON.parse(metadata) as { presenterNotes?: unknown };
    return Array.isArray(obj?.presenterNotes)
      ? obj.presenterNotes.map((n) => (typeof n === "string" ? n : ""))
      : [];
  } catch {
    return [];
  }
}

/** 设计项目：顶层容器。 */
export interface DesignProject {
  id: string;
  title: string;
  description?: string;
  color?: string;
  defaultSystemId?: string;
  haProjectId?: string;
  sessionId?: string;
  agentId?: string;
  createdAt: string;
  updatedAt: string;
  artifactCount: number;
  /** 待复查（needs_review）产物数（列表状态徽标用，读取时聚合）。 */
  needsReviewCount?: number;
  metadata?: string;
}

/** 单个可交付产物。 */
export interface DesignArtifact {
  id: string;
  projectId: string;
  title: string;
  kind: ArtifactKind;
  systemId?: string;
  status: ArtifactStatus;
  viewportW?: number;
  viewportH?: number;
  currentVersion: number;
  critiqueScore?: number;
  thumbnailPath?: string;
  createdAt: string;
  updatedAt: string;
  metadata?: string;
  /** 所属文件夹（页面分组）：斜杠路径，空串 = 根。 */
  folder?: string;
}

/** 产物 + 已解析预览路径（`get_design_artifact_cmd` 返回）。 */
export interface DesignArtifactView extends DesignArtifact {
  artifactPath: string;
  /** 当前 body.html 的 BLAKE3（可视化编辑 stale-write 守卫）。 */
  bodyHash: string;
}

/** iframe bridge 回传的选中元素信息（`ds_selected`）。 */
export interface DesignSelectedElement {
  oid: string;
  tag: string;
  styles: Record<string, string>;
  /** B5：<a>/<img> 的可编辑属性（href/src/alt）；其它 tag 为空。 */
  attrs?: Record<string, string>;
  text: string;
  isLeaf: boolean;
  rect: { x: number; y: number; w: number; h: number };
}

/** 元素锚定的批注钉（`design_comment_*_cmd`）。 */
export interface DesignComment {
  id: number;
  artifactId: string;
  /** 锚定元素的 data-ds-oid；脱锚为 null/undefined。 */
  oid?: number | null;
  relX: number;
  relY: number;
  tag?: string;
  snippet?: string;
  body: string;
  resolved: boolean;
  createdAt: string;
}

/** bridge 在批注模式下点选元素落钉时回传父窗的锚点信息。 */
export interface CommentPlacement {
  oid: number | null;
  relX: number;
  relY: number;
  tag?: string;
  snippet?: string;
  /** 套选（Wave 2-⑪）：命中的多个成员元素（一条批注覆盖多元素，成员随批注带给 AI）。 */
  members?: { oid: number; tag: string; snippet: string }[];
}

/** 5 维质量评审结果（`critique_design_artifact_cmd`）。 */
export interface CritiqueResult {
  brand: number;
  accessibility: number;
  hierarchy: number;
  usability: number;
  performance: number;
  overall: number;
  summary: string;
  fixes: string[];
}

/** 可视化微调回写入参（`patch_design_element_cmd`）。 */
export interface ElementPatchInput {
  artifactId: string;
  oid: number;
  text?: string;
  styles?: [string, string][];
  /** B5 属性编辑（href/src/alt）；空值 = 清除该属性。 */
  attrs?: [string, string][];
  expectedHash?: string;
}

/** 产物版本快照元数据。 */
/** 版本溯源标签：AI 生成/精修、手动编辑/换系统、回滚。 */
export type VersionOrigin = "ai" | "manual" | "restore";

export interface DesignArtifactVersion {
  id: number;
  artifactId: string;
  versionNumber: number;
  message?: string;
  critiqueScore?: number;
  /** 溯源标签（后端派生；旧版本行可能为空）。 */
  origin?: VersionOrigin;
  /** 该版本对应的生成 prompt 摘要（仅 AI 版本有）。 */
  promptSummary?: string;
  createdAt: string;
}

/** 设计系统索引元数据。 */
export interface DesignSystemMeta {
  id: string;
  name: string;
  slug: string;
  source: "builtin" | "user" | "extracted";
  /** 分组类目（品牌品类 / 原创原型），仅用于选择器分组；用户系统为 undefined。 */
  category?: string;
  summary?: string;
  thumbnailPath?: string;
  createdAt: string;
  updatedAt: string;
}

/** 设计空间配置。 */
export interface DesignConfig {
  enabled: boolean;
  autoShow: boolean;
  defaultSystemId?: string;
  autoCritique: boolean;
  maxVersionsPerArtifact: number;
  panelWidth: number;
  selfCheck: boolean;
  /** 反向提取图片大小上限（MB）。0 = 不限。默认 24。 */
  maxExtractImageMb: number;
  /** 导出栅格化倍率（清晰度），[1,4]。默认 2。 */
  exportScale: number;
  /** PDF 导出 JPEG 质量（1–100），[40,100]。默认 92。 */
  exportJpegQuality: number;
}

/** 设计模板（recipe）：某形态的常见场景，供首屏模板快选。 */
export interface DesignRecipe {
  id: string;
  name: string;
  kind: ArtifactKind;
  scenario: string;
  summary: string;
  guidance: string;
}

/** 创建项目入参。 */
export interface CreateProjectInput {
  title: string;
  description?: string;
  color?: string;
  defaultSystemId?: string;
  haProjectId?: string;
}

/** 创建产物入参。 */
export interface CreateArtifactInput {
  projectId: string;
  title: string;
  kind: ArtifactKind;
  systemId?: string;
  bodyHtml?: string;
  css?: string;
  js?: string;
  /** 生成用一句话 brief：image 走 image_generate；其余形态走模型一次生成自包含设计。 */
  prompt?: string;
  /** 参考图 base64（「照着这张图生成匹配产物」）：非媒体形态经 vision 描述后走生成管线。 */
  referenceImageB64?: string;
  referenceImageMime?: string;
}

/** 产物形态元数据（前端展示：标签 + 图标语义）。 */
export const ARTIFACT_KINDS: ArtifactKind[] = [
  "web",
  "mobile",
  "deck",
  "dashboard",
  "poster",
  "document",
  "email",
  "image",
  "motion",
  "audio",
  "component",
];

/** 设计系统正文（`get_design_system_cmd` 返回）。 */
export interface DesignSystemFull {
  meta: DesignSystemMeta;
  systemMd: string;
  tokens: Record<string, string>;
  /** 提取时 harvest 的 logo/配图资产（data-uri）；非提取系统为空。套件视图后端渲染时消费。 */
  assets?: { logos: string[]; images: string[] };
}

/** 反向提取入参（`extract_design_system_cmd`）。 */
export interface ExtractSystemInput {
  name: string;
  from: "brief" | "codebase" | "url" | "image";
  brief?: string;
  path?: string;
  url?: string;
}

/** 设计方向候选（`propose_design_directions_cmd`）。 */
export interface DesignDirection {
  name: string;
  summary: string;
  tokens: Record<string, string>;
}

/** 多平台 Token 导出产物（`export_design_tokens_cmd`）。 */
export interface TokenExport {
  format: "css" | "scss" | "ts" | "swift" | "android" | "dtcg";
  label: string;
  filename: string;
  language: string;
  content: string;
}

/** 设计系统 → 代码工程绑定（工程轴 D）。 */
export interface DesignCodeBinding {
  id: number;
  systemId: string;
  targetDir: string;
  subfolder: string;
  formats: string[];
  createdAt: string;
  lastSyncedAt?: string;
}

/** 同步结果（`sync_design_code_binding_cmd`）。 */
export interface BindingSyncReport {
  bindingId: number;
  dir: string;
  written: string[];
  syncedAt: string;
}

/** A design-space per-project chat thread — one row per `kind='design'` session,
 *  joined with session metadata for the history picker. Mirrors `KbChatThread`. */
export interface DesignChatThread {
  sessionId: string;
  projectId: string;
  /** Agent baked into this thread — restored on history-picker switch so
   *  follow-ups run with the thread's own agent + model. */
  agentId: string;
  title?: string | null;
  /** Thread creation time (epoch ms). */
  createdAt: number;
  /** Session `updated_at` (rfc3339) — recency sort key. */
  updatedAt: string;
  messageCount: number;
  lastSnippet?: string | null;
}
