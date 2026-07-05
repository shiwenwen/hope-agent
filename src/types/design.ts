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
  | "motion";

/** 产物生成状态。 */
export type ArtifactStatus = "planned" | "generating" | "ready" | "failed";

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
  text: string;
  isLeaf: boolean;
  rect: { x: number; y: number; w: number; h: number };
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
  expectedHash?: string;
}

/** 产物版本快照元数据。 */
export interface DesignArtifactVersion {
  id: number;
  artifactId: string;
  versionNumber: number;
  message?: string;
  critiqueScore?: number;
  createdAt: string;
}

/** 设计系统索引元数据。 */
export interface DesignSystemMeta {
  id: string;
  name: string;
  slug: string;
  source: "builtin" | "user" | "extracted";
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
  /** 反向提取专用视觉模型（providerId:modelId）。空 = 复用活跃模型。 */
  extractVisionModel?: string;
  /** 质量评审专用模型（providerId:modelId）。空 = 复用默认分析模型。 */
  critiqueModel?: string;
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
];

/** 设计系统正文（`get_design_system_cmd` 返回）。 */
export interface DesignSystemFull {
  meta: DesignSystemMeta;
  systemMd: string;
  tokens: Record<string, string>;
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
