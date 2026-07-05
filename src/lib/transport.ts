/**
 * Transport abstraction layer.
 *
 * Provides a unified interface for frontend code to communicate with the
 * backend regardless of whether it runs inside Tauri (IPC) or as a
 * standalone web app (HTTP / WebSocket).
 */

import type { FileChangesMetadata, MediaItem, SandboxMode, SessionMode } from "@/types/chat";

/**
 * One entry in `ChatStartArgs.attachments`. Snake-case fields are the
 * wire form — both Tauri IPC and `POST /api/chat` serialize this object
 * as-is. `data` and `file_path` are mutually-exclusive: inline images
 * carry base64 in `data`; everything else is persisted to disk and
 * referenced by absolute `file_path`.
 */
export interface ChatAttachment {
  name: string;
  mime_type: string;
  source?: "upload" | "mention" | "plan_mention" | "quote";
  data?: string;
  file_path?: string;
  /** For `source: "quote"`: 1-based line range of the quoted snippet ("12-20"). */
  quote_lines?: string;
}

/**
 * Args accepted by {@link Transport.startChat}. Mirrors the parameters of
 * the Tauri `chat` command and the `POST /api/chat` body.
 */
export interface ChatStartArgs {
  message: string;
  attachments: ReadonlyArray<ChatAttachment>;
  sessionId: string | null;
  incognito?: boolean;
  modelOverride?: string;
  agentId?: string;
  permissionMode?: SessionMode;
  sandboxMode?: SandboxMode;
  planMode?: string;
  temperatureOverride?: number;
  reasoningEffort?: string;
  displayText?: string;
  /** Marks the user message as a Plan Mode approve/resume trigger so the
   *  backend stamps `attachments_meta = {plan_trigger: true}` and the UI
   *  renders it as a system chip instead of a regular user bubble. */
  isPlanTrigger?: boolean;
  /** Structured payload for plan inline-comment messages. Backend stamps
   *  `attachments_meta = {plan_comment: {...}}`; the desktop GUI uses it to
   *  render PlanCommentBubble instead of the markdown displayText. */
  planComment?: { selectedText: string; comment: string };
  workingDir?: string | null;
  /** Lazy project binding. When a project draft (no session yet) sends its first
   *  message, this carries the project id so the backend auto-create branch
   *  materializes the session inside the project. Ignored for existing-session
   *  sends; mutually exclusive with incognito (coerced server-side). */
  projectId?: string | null;
  /** Composer-staged KB attaches. The backend applies them on the auto-create
   *  branch (mirrors workingDir), before the first turn runs, so the first
   *  message already sees the access. Ignored for existing-session sends. */
  kbAttachments?: { kbId: string; access: string }[];
  /** Tool-visibility scope. `"knowledge"` trims the injected tool set to the
   *  knowledge-space white-list (note read/write + recall + framework basics);
   *  set by the knowledge-space sidebar chat. Omit for normal chats. */
  toolScope?: "knowledge";
  // Tauri's invoke serializes extra unknown fields without complaint, and
  // HTTP's POST body is plain JSON — keep this open so HTTP impl can
  // pass-through without an unsafe `as Record<string, unknown>` cast.
  [key: string]: unknown;
}

/**
 * Transport defines the three communication primitives the app needs:
 *
 * 1. `call` – request/response (command invocation)
 * 2. `startChat` – run a chat turn and stream events for it
 * 3. `listen` – subscribe to backend-pushed events
 */
export interface Transport {
  /**
   * Invoke a backend command and return the result.
   *
   * In Tauri mode this maps to `invoke()`.
   * In HTTP mode this maps to REST endpoints.
   */
  call<T>(command: string, args?: Record<string, unknown>): Promise<T>;

  /**
   * Prepare file data for transport.
   *
   * Returns a `Blob` (HTTP — multipart, zero-copy) or `number[]`
   * (Tauri IPC — JSON serialization). Callers pass the result as the
   * `data` field in `call()` args.
   */
  prepareFileData(buffer: ArrayBuffer, mimeType: string): Blob | number[];

  /**
   * Run a chat turn and stream its events. Resolves with the final
   * assistant response text once the turn completes.
   *
   * - Tauri mode: opens a `Channel<string>` internally and forwards each
   *   delta to `onEvent`.
   * - HTTP mode: stream deltas arrive via the global EventBus path
   *   (`/ws/events` → `chat:stream_delta`, consumed by
   *   `useChatStreamReattach`). `onEvent` is invoked only for a
   *   synthesized `session_created` event so callers' `__pending__` cache
   *   gets renamed in place.
   */
  startChat(args: ChatStartArgs, onEvent: (event: string) => void): Promise<string>;

  /**
   * Subscribe to a named backend event.
   *
   * @returns An unsubscribe function.
   */
  listen(eventName: string, handler: (payload: unknown) => void): () => void;

  /**
   * Resolve a {@link MediaItem} into a URL that `<img src>` / `<a href>` /
   * window.open can consume. Returns `null` when the item isn't reachable
   * in the current transport (legacy URL shape, missing `localPath` in
   * Tauri mode, etc.) — callers should render a FileCard fallback instead
   * of a broken `<img src="">`.
   */
  resolveMediaUrl(item: MediaItem): string | null;

  /**
   * Resolve a persisted image reference (avatar path, project-logo data URL,
   * remote image URL) into something `<img src>` can consume in the current
   * transport.
   *
   * Accepted inputs:
   *  - `data:` URL  → passthrough (works in both modes)
   *  - `http(s)://` URL → passthrough
   *  - Absolute filesystem path (typical for avatars, e.g.
   *    `~/.hope-agent/avatars/foo.png`):
   *       - Tauri mode → wrapped via `convertFileSrc`
   *       - HTTP mode  → rewritten to a server route
   *         (`/api/avatars/{basename}?token=...`) when the path's parent
   *         directory matches a known asset category; otherwise `null`
   *  - `null` / empty string → `null`
   *
   * Callers should fall back to their emoji / initials / default-icon
   * rendering when the result is `null`.
   */
  resolveAssetUrl(path: string | null | undefined): string | null;

  /**
   * Trigger the user-facing "open" action for a media item.
   * - Tauri: opens the file with the OS default handler.
   * - HTTP: opens the server route in a new browser tab; previewable MIME
   *   types render inline and other files fall back to attachment download.
   */
  openMedia(item: MediaItem): Promise<void>;

  /** Trigger a download for a media item even when its MIME type is previewable. */
  downloadMedia(item: MediaItem): Promise<void>;

  /**
   * Open a filesystem path referenced by a session message.
   * - Tauri: opens the local path with the OS default handler.
   * - HTTP: serves it through a session-scoped, history-authorized route.
   */
  openFilePath(path: string, opts?: { sessionId?: string | null }): Promise<void>;

  /** Download a filesystem path referenced by a session message. */
  downloadFilePath(
    path: string,
    opts?: { sessionId?: string | null; filename?: string },
  ): Promise<void>;

  /**
   * Show the media file in the OS file manager (Finder / Explorer).
   * No-op on HTTP/Web — UIs should gate on {@link supportsLocalFileOps}.
   */
  revealMedia(item: MediaItem): Promise<void>;

  /**
   * Whether the transport supports local-file ops (open-in-app, reveal-in-folder).
   * False in HTTP/Web mode — UIs should hide the "Reveal" action.
   */
  supportsLocalFileOps(): boolean;

  /**
   * Prompt the user to pick a local image and return a {@link PickedImage}
   * the caller can feed into a preview / crop dialog. Returns `null` when
   * the user cancels. See {@link PickLocalImageFn} for transport-specific
   * behaviour.
   */
  pickLocalImage(): Promise<PickedImage | null>;

  /**
   * Prompt the user to pick a directory and return its absolute path.
   * Returns `null` when the user cancels.
   *
   * - Tauri mode: opens the native directory picker via
   *   `@tauri-apps/plugin-dialog`. The returned path is on the user's local
   *   filesystem.
   * - HTTP mode: the implementation is expected to surface a server-side
   *   directory browser (see {@link listServerDirectory}); the returned path
   *   is on the server machine, not the browser client.
   */
  pickLocalDirectory(): Promise<string | null>;

  /**
   * List a single directory level on the server machine. Used by both the
   * directory-browser modal (HTTP mode) and the chat-input `@` mention popper
   * (path-mode navigation, both modes).
   *
   * @param path Absolute path to list. When omitted, the server returns a
   *             platform default root (`/` on Unix, user profile on Windows).
   */
  listServerDirectory(path?: string): Promise<DirListing>;

  /**
   * Fuzzy-search files & directories under `root`. Used by the chat-input
   * `@` mention popper for tokens that don't contain a `/` (e.g. `@chat`).
   *
   * - `root` MUST be absolute.
   * - `q` is matched as a case-insensitive subsequence against name then path.
   * - `limit` defaults to 50 server-side; results are pre-sorted by score.
   */
  searchFiles(root: string, q: string, limit?: number): Promise<FileSearchResponse>;

  /**
   * Export a session's conversation to a file in Markdown / JSON / HTML.
   *
   * - Tauri mode: opens the native save dialog, lets the user pick a path,
   *   then writes the file via `export_session_cmd`. Returns `{ savedPath,
   *   filename }`.
   * - HTTP mode: streams the response from
   *   `GET /api/sessions/{id}/export` and returns a `Blob` plus the
   *   server-supplied filename. Caller is expected to trigger a browser
   *   download via `URL.createObjectURL` + `<a download>`.
   *
   * Returns `null` only when the user cancels the save dialog (Tauri).
   * HTTP mode never returns null on success — failures throw.
   */
  exportSession(args: ExportSessionArgs): Promise<ExportSessionResult | null>;

  /**
   * Build a URL / asset-src for raw file bytes inside a project/session
   * workspace — used for image & PDF preview and downloads.
   *
   * - Tauri: resolves the absolute path via `project_fs_resolve` and returns
   *   an `asset://` src from `convertFileSrc`.
   * - HTTP: returns a tokened `/api/fs/raw` URL usable directly in
   *   `<img>` / `<iframe>`.
   *
   * Returns `null` when the path can't be resolved.
   */
  projectFsRawUrl(
    args: ProjectFsScope & { path: string; download?: boolean },
  ): Promise<string | null>;

  /**
   * Read a file's text content for the in-app preview panel, by **absolute
   * path** (the path-based sibling of {@link ProjectFsApi.readFile}). Binary /
   * oversized files come back `isBinary: true`.
   * - Tauri: `preview_read_text` (trusts local paths).
   * - HTTP: `GET /api/sessions/{id}/files/read`, gated by session reference +
   *   working-dir containment. Requires `sessionId`; throws without one.
   */
  previewReadText(
    path: string,
    opts?: { sessionId?: string | null },
  ): Promise<FileTextContent>;

  /**
   * Extract a PDF / Office document for the in-app preview panel, by absolute
   * path. Same modes / authorization as {@link previewReadText}.
   */
  previewExtractDoc(
    path: string,
    opts?: { sessionId?: string | null },
  ): Promise<ExtractedContent>;

  /**
   * Resolve a raw URL for an absolute file path, for `<img>` / `<iframe>` /
   * `<video>` / `<audio>` preview (and binary-placeholder open/download).
   * - Tauri: `resolveAssetUrl(path)` (`convertFileSrc`); `download` is ignored.
   * - HTTP: a tokened `/api/sessions/{id}/files/by-path` URL (with `download=1`
   *   when requested). Returns `null` without a `sessionId`.
   */
  previewRawUrl(
    path: string,
    opts?: { sessionId?: string | null },
    download?: boolean,
  ): Promise<string | null>;

  /**
   * Upload a single file into a workspace directory. Multipart on HTTP;
   * `invoke` with a byte array on Tauri.
   */
  projectFsUpload(
    args: ProjectFsScope & {
      dirPath: string;
      data: Blob;
      fileName: string;
      mimeType?: string;
      overwrite?: boolean;
    },
  ): Promise<UploadResult>;

  /**
   * Aggregate the session's workspace artifacts (files touched + URL sources)
   * over its FULL history — the complete set the workspace panel merges with
   * its in-memory live tail. Summary only (no diff snapshots).
   * - Tauri: `load_session_artifacts_cmd`.
   * - HTTP: `GET /api/sessions/{id}/artifacts`.
   */
  loadSessionArtifacts(sessionId: string): Promise<SessionArtifacts>;

  /**
   * Read-only environment snapshot for the workspace panel. This is UI-only
   * context; it is not injected into the model prompt.
   */
  loadSessionEnvironment(sessionId: string): Promise<WorkspaceEnvironmentSnapshot>;

  /**
   * Read the current Git working-tree diff for the session workspace, relative
   * to HEAD. Returns the same `file_changes` shape emitted by file-mutating
   * tools so the DiffPanel can render it directly.
   */
  loadSessionGitDiff(sessionId: string): Promise<FileChangesMetadata>;
}

/**
 * Args for {@link Transport.exportSession}.
 */
export interface ExportSessionArgs {
  sessionId: string;
  /** Default filename to suggest in the save dialog (Tauri) — extension
   *  derived from `format` if omitted. */
  defaultFilename?: string;
  format: "md" | "json" | "html";
  includeThinking: boolean;
  includeTools: boolean;
}

/**
 * Result of {@link Transport.exportSession}. Exactly one of `savedPath`
 * (Tauri) or `blob` (HTTP) is set.
 */
export interface ExportSessionResult {
  filename: string;
  savedPath?: string;
  blob?: Blob;
}

/**
 * One level of a server-side directory listing. Shape mirrors the
 * `/api/filesystem/list-dir` response.
 */
export interface DirListing {
  path: string;
  parent: string | null;
  entries: DirEntry[];
  /** `true` when the server capped results; show a "results truncated" hint. */
  truncated: boolean;
}

export interface DirEntry {
  name: string;
  /** Absolute path of this entry, pre-joined by the server. */
  path: string;
  isDir: boolean;
  isSymlink: boolean;
  size: number | null;
  modifiedMs: number | null;
}

/** A single result from `searchFiles`, sorted by score descending. */
export interface FileMatch {
  name: string;
  /** Absolute path. */
  path: string;
  /** Path relative to the search root, with `/` separator. */
  relPath: string;
  isDir: boolean;
  /** Path-aware fuzzy score; higher = better. Server-sorted. */
  score: number;
}

/**
 * Response from `searchFiles`. Mirrors the `/api/filesystem/search-files`
 * response shape.
 */
export interface FileSearchResponse {
  /** Canonicalized absolute root that was searched. */
  root: string;
  matches: FileMatch[];
  /** `true` when the walk hit the per-search file cap and stopped early. */
  truncated: boolean;
}

// -- Project file browser (workspace-scoped filesystem) ----------------------

/** Selects which working directory the file-browser API operates on. */
export interface ProjectFsScope {
  /** `"session"` / `"project"` resolve a session/project working dir; `"path"`
   *  is a read-only worktree jump whose `scopeId` is an encoded triple
   *  `base_scope ∣ base_scope_id ∣ target_abs` (see `FileBrowserView`'s
   *  `encodePathScope`), validated server-side against the base repo's worktree
   *  list so it can only reach the current repo's own worktrees. */
  scope: "session" | "project" | "path";
  scopeId: string;
}

/** One entry in a workspace directory listing. Paths are relative to the
 *  workspace root (`/`-separated). */
export interface WorkspaceEntry {
  name: string;
  relPath: string;
  isDir: boolean;
  isSymlink: boolean;
  size: number | null;
  modifiedMs: number | null;
}

export interface WorkspaceListing {
  /** Listed directory, relative to the workspace root (`""` = root). */
  dirRel: string;
  parentRel: string | null;
  entries: WorkspaceEntry[];
  truncated: boolean;
}

export interface FileTextContent {
  relPath: string;
  content: string;
  /** `true` for binary/oversized files — `content` is empty, use the raw URL. */
  isBinary: boolean;
  mime: string | null;
  totalLines: number;
  sizeBytes: number;
  truncated: boolean;
}

/** One rendered page (PDF) or embedded image (Office), base64-encoded. */
export interface ExtractedImageDto {
  data: string;
  mime: string;
  label: string;
}

export interface ExtractedContent {
  relPath: string;
  /** `"pdf"` or `"office"`. */
  kind: string;
  text: string | null;
  images: ExtractedImageDto[];
}

/**
 * Backend-aggregated file summary — the `diff`-less sibling of
 * `SessionFileEntry`. The workspace panel maps it back with `diff: null`
 * (window-外 files have no historical diff; they preview current content).
 */
export interface FileArtifactSummary {
  path: string;
  kind: "modified" | "read";
  linesAdded: number;
  linesRemoved: number;
  readLines: number | null;
}

/** Backend-aggregated URL source (mirror of `SessionUrlSource`). */
export interface UrlSourceDto {
  url: string;
  origin: "web_search" | "message";
}

/**
 * Full-session workspace artifacts aggregated server-side over the whole
 * message history. `*Truncated` flags whether the list was capped (most-recent
 * 1000). See {@link Transport.loadSessionArtifacts}.
 */
export interface SessionArtifacts {
  files: FileArtifactSummary[];
  sources: UrlSourceDto[];
  filesTruncated: boolean;
  sourcesTruncated: boolean;
}

export type WorkspaceWorkingDirSource = "session" | "project" | "projectDefault" | "none";

export interface WorkspaceWorkingDirSnapshot {
  path: string | null;
  source: WorkspaceWorkingDirSource;
  exists: boolean;
  name: string | null;
}

export type WorkspaceGitSyncState =
  | "upToDate"
  | "ahead"
  | "behind"
  | "diverged"
  | "noUpstream"
  | "unknown";

export interface WorkspaceGitStatus {
  changedFiles: number;
  stagedFiles: number;
  unstagedFiles: number;
  untrackedFiles: number;
  conflictedFiles: number;
  linesAdded: number;
  linesRemoved: number;
  clean: boolean;
}

export interface WorkspaceGitSync {
  upstream: string | null;
  remote: string | null;
  ahead: number;
  behind: number;
  state: WorkspaceGitSyncState;
}

export interface WorkspaceGitCommit {
  hash: string;
  subject: string;
}

export interface WorkspaceGitSnapshot {
  root: string;
  branch: string | null;
  detached: boolean;
  head: string | null;
  worktrees: WorktreeInfo[];
  status: WorkspaceGitStatus;
  sync: WorkspaceGitSync;
  lastCommit: WorkspaceGitCommit | null;
}

export interface WorkspaceEnvironmentSnapshot {
  workingDir: WorkspaceWorkingDirSnapshot;
  git: WorkspaceGitSnapshot | null;
}

export interface WriteResult {
  relPath: string;
  sizeBytes: number;
}

export interface RenameResult {
  relPath: string;
}

export interface UploadResult {
  relPath: string;
  sizeBytes: number;
}

export interface WorktreeInfo {
  path: string;
  branch: string | null;
  isCurrent: boolean;
}

export type ManagedWorktreeState = "active" | "archived" | "handoff";
export type ManagedWorktreePurpose = "manual" | "workflow" | "subagent";

export interface ManagedWorktreeDirtySnapshot {
  clean: boolean;
  stagedFiles: number;
  unstagedFiles: number;
  untrackedFiles: number;
  conflictedFiles: number;
  changedFiles: number;
}

export interface ManagedWorktree {
  id: string;
  sessionId: string;
  childSessionId?: string | null;
  workflowRunId?: string | null;
  purpose: ManagedWorktreePurpose;
  state: ManagedWorktreeState;
  label?: string | null;
  repoRoot: string;
  sourceWorkingDir: string;
  path: string;
  baseRef?: string | null;
  baseBranch?: string | null;
  baseSha?: string | null;
  gitBranch?: string | null;
  dirtySnapshot?: ManagedWorktreeDirtySnapshot | null;
  pathExists: boolean;
  createdAt: string;
  updatedAt: string;
  archivedAt?: string | null;
  restoredAt?: string | null;
  handedOffAt?: string | null;
}

export interface LspRange {
  startLine: number;
  startColumn: number;
  endLine: number;
  endColumn: number;
}

export interface LspDiagnostic {
  uri: string;
  path?: string | null;
  range: LspRange;
  severity: "error" | "warning" | "information" | "hint" | "unknown";
  code?: string | null;
  source?: string | null;
  message: string;
}

export interface LspDiagnosticsSnapshot {
  sessionId: string;
  workspaceRoot?: string | null;
  diagnostics: LspDiagnostic[];
  files: number;
  errors: number;
  warnings: number;
}

export interface LspServerInfo {
  id: string;
  command: string;
  args: string[];
  available: boolean;
  extensions: string[];
  workspaceRoot?: string | null;
  active: boolean;
  openDocuments: number;
  diagnosticFiles: number;
}

export interface LspStatusSnapshot {
  sessionId: string;
  workspaceRoot?: string | null;
  servers: LspServerInfo[];
}

export type ContextCandidateKind =
  | "file"
  | "symbol"
  | "diagnostic"
  | "review_finding"
  | "verification_step"
  | "goal_evidence"
  | "task"
  | "workflow_op"
  | "ide_context"
  | "url_source"
  | "document"
  | "email_thread"
  | "calendar_event"
  | "sheet_range"
  | "knowledge_note"
  | "web_source"
  | "decision"
  | "artifact";

export interface ContextCandidate {
  id: string;
  kind: ContextCandidateKind;
  title: string;
  subtitle?: string | null;
  path?: string | null;
  line?: number | null;
  url?: string | null;
  score: number;
  reasons: string[];
  sources: string[];
  status?: string | null;
  metadata: Record<string, unknown>;
}

export interface ContextRetrievalStats {
  gitChanges: number;
  artifactFiles: number;
  diagnostics: number;
  reviewFindings: number;
  verificationSteps: number;
  goalEvidence: number;
  tasks: number;
  workflowOps: number;
  ideContextSignals: number;
  fileSearchMatches: number;
  symbols: number;
  urlSources: number;
  domainCandidates: number;
  domainEvidence: number;
  accessIssues: number;
  warnings: string[];
}

export interface DomainContextProfile {
  domain: string;
  templateId?: string | null;
  templateVersion?: string | null;
  templateTitle?: string | null;
  taskType?: string | null;
  goalId?: string | null;
  goalObjective?: string | null;
  completionCriteria?: string | null;
  requiredEvidence: DomainEvidenceRequirement[];
  approvalGates: DomainApprovalGate[];
  verificationPolicy: DomainVerificationRule[];
  source: string;
}

export interface ContextAccessIssue {
  kind: string;
  title: string;
  reason: string;
  requiredConnector?: string | null;
  domain?: string | null;
  action: string;
}

export interface ContextRetrievalSnapshot {
  sessionId: string;
  query?: string | null;
  workspaceRoot?: string | null;
  candidates: ContextCandidate[];
  stats: ContextRetrievalStats;
  domainContext?: DomainContextProfile | null;
  accessIssues: ContextAccessIssue[];
  truncated: boolean;
  disabledReason?: string | null;
  generatedAt: string;
}

export interface IdeLineRange {
  path?: string | null;
  startLine?: number | null;
  endLine?: number | null;
  text?: string | null;
}

export interface IdeDiagnosticContext {
  path?: string | null;
  line?: number | null;
  severity?: string | null;
  message?: string | null;
}

export interface IdeSymbolContext {
  name?: string | null;
  kind?: string | null;
  path?: string | null;
  line?: number | null;
}

export interface SessionIdeContext {
  source?: string | null;
  currentFile?: string | null;
  selection?: IdeLineRange | null;
  openTabs?: string[];
  activeDiagnostic?: IdeDiagnosticContext | null;
  activeSymbol?: IdeSymbolContext | null;
}

export interface SessionIdeContextSnapshot {
  sessionId: string;
  context: SessionIdeContext;
  updatedAt: string;
}

export type ReviewRunState = "running" | "completed" | "failed" | "cancelled";
export type ReviewSeverity = "p0" | "p1" | "p2" | "p3";
export type ReviewVerdict = "confirmed" | "plausible" | "refuted";
export type ReviewFindingStatus = "open" | "resolved" | "dismissed" | "false_positive";

export interface ReviewRun {
  id: string;
  sessionId: string;
  scope: string;
  state: ReviewRunState;
  baseRef?: string | null;
  goalId?: string | null;
  summary: string;
  stats: Record<string, unknown>;
  error?: string | null;
  createdAt: string;
  updatedAt: string;
  completedAt?: string | null;
}

export interface ReviewFinding {
  id: string;
  runId: string;
  sessionId: string;
  file: string;
  startLine?: number | null;
  endLine?: number | null;
  title: string;
  body: string;
  category: string;
  severity: ReviewSeverity;
  verdict: ReviewVerdict;
  status: ReviewFindingStatus;
  evidence: Record<string, unknown>;
  createdAt: string;
  updatedAt: string;
  resolvedAt?: string | null;
}

export interface ReviewEvent {
  id: number;
  runId: string;
  seq: number;
  kind: string;
  payload: unknown;
  createdAt: string;
}

export interface ReviewRunSnapshot {
  run: ReviewRun;
  findings: ReviewFinding[];
  events: ReviewEvent[];
}

export type VerificationRunState = "planned" | "running" | "completed" | "failed" | "cancelled";
export type VerificationStepState = "pending" | "running" | "passed" | "failed" | "skipped" | "timed_out";
export type VerificationRisk = "low" | "medium" | "high";

export interface VerificationRun {
  id: string;
  sessionId: string;
  scope: string;
  state: VerificationRunState;
  goalId?: string | null;
  summary: string;
  stats: Record<string, unknown>;
  error?: string | null;
  createdAt: string;
  updatedAt: string;
  completedAt?: string | null;
}

export interface VerificationStep {
  id: string;
  runId: string;
  sessionId: string;
  seq: number;
  command: string;
  cwd: string;
  title: string;
  reason: string;
  category: string;
  risk: VerificationRisk;
  autoRun: boolean;
  state: VerificationStepState;
  exitCode?: number | null;
  outputPreview?: string | null;
  durationMs?: number | null;
  createdAt: string;
  updatedAt: string;
  startedAt?: string | null;
  completedAt?: string | null;
}

export interface VerificationEvent {
  id: number;
  runId: string;
  seq: number;
  kind: string;
  payload: unknown;
  createdAt: string;
}

export interface VerificationRunSnapshot {
  run: VerificationRun;
  steps: VerificationStep[];
  events: VerificationEvent[];
}

export type DomainQualityRunState =
  | "running"
  | "completed"
  | "failed"
  | "blocked"
  | "needs_user"
  | "cancelled";
export type DomainQualitySeverity = "p0" | "p1" | "p2" | "p3";
export type DomainQualityCheckStatus =
  | "passed"
  | "failed"
  | "blocked"
  | "needs_user"
  | "advisory";

export interface DomainQualityRun {
  id: string;
  sessionId: string;
  goalId?: string | null;
  domain: string;
  templateId?: string | null;
  templateVersion?: string | null;
  state: DomainQualityRunState;
  summary: string;
  stats: Record<string, unknown>;
  error?: string | null;
  createdAt: string;
  updatedAt: string;
  completedAt?: string | null;
}

export interface DomainQualityCheck {
  id: string;
  runId: string;
  sessionId: string;
  seq: number;
  checkType: string;
  profile: string;
  title: string;
  body: string;
  severity: DomainQualitySeverity;
  status: DomainQualityCheckStatus;
  evidenceType?: string | null;
  sourceMetadata: Record<string, unknown>;
  createdAt: string;
  updatedAt: string;
}

export interface DomainQualityEvent {
  id: number;
  runId: string;
  seq: number;
  kind: string;
  payload: unknown;
  createdAt: string;
}

export interface DomainQualityRunSnapshot {
  run: DomainQualityRun;
  checks: DomainQualityCheck[];
  events: DomainQualityEvent[];
}

export interface RunDomainQualityInput {
  sessionId: string;
  goalId?: string | null;
  domain?: string | null;
  templateId?: string | null;
  templateVersion?: string | null;
  profiles?: string[];
  artifactTitle?: string | null;
  artifactKind?: string | null;
  sourceMetadata?: Record<string, unknown>;
  explicitUserApproval?: boolean;
}

export interface DomainEvalTaskInput {
  prompt: string;
  fixtureKind: string;
  sourceRequirements: string[];
}

export interface DomainEvalEvidenceRequirement {
  evidenceType: string;
  title: string;
  required: boolean;
  minCount: number;
  metadataKeys: string[];
}

export interface DomainEvalCalibrationRecord {
  id?: string | null;
  taskId?: string | null;
  taskVersion?: string | null;
  domain?: string | null;
  projectId?: string | null;
  scope?: string | null;
  verdict?: string | null;
  sourceRunId?: string | null;
  calibratedAt: string;
  reviewer: string;
  note: string;
}

export interface DomainEvalTask {
  id: string;
  version: string;
  domain: string;
  title: string;
  taskType: string;
  input: DomainEvalTaskInput;
  allowedTools: string[];
  requiredEvidence: DomainEvalEvidenceRequirement[];
  successCriteria: string[];
  prohibitedActions: string[];
  calibration: DomainEvalCalibrationRecord[];
}

export interface ListDomainEvalTasksInput {
  domain?: string | null;
  projectId?: string | null;
  limit?: number | null;
}

export interface RecordDomainEvalCalibrationInput {
  taskId: string;
  taskVersion?: string | null;
  projectId?: string | null;
  reviewer?: string | null;
  verdict: string;
  note: string;
  sourceRunId?: string | null;
}

export interface ListDomainEvalCalibrationsInput {
  taskId?: string | null;
  domain?: string | null;
  projectId?: string | null;
  includeUserScope?: boolean;
  limit?: number | null;
}

export interface RunDomainEvalTaskInput {
  sessionId: string;
  taskId: string;
  label?: string | null;
  sourceQualityRunId?: string | null;
  sourceType?: string | null;
}

export interface RunDomainEvalFixtureInput {
  fixture: DomainEvalFixture;
}

export interface DomainEvalFixture {
  name: string;
  description?: string;
  taskId: string;
  label?: string | null;
  executionMode?: string;
  domain?: string | null;
  goal?: DomainEvalFixtureGoal;
  evidence?: DomainEvalFixtureEvidence[];
  workflow?: DomainEvalFixtureWorkflow | null;
  quality?: DomainEvalFixtureQuality | null;
  execution?: DomainEvalFixtureExecution;
  checks?: DomainEvalFixtureChecks;
}

export interface DomainEvalFixtureGoal {
  objective?: string | null;
  completionCriteria?: string | null;
  workflowTemplateId?: string | null;
  workflowTemplateVersion?: string | null;
  workflowTaskType?: string | null;
}

export interface DomainEvalFixtureEvidence {
  evidenceType: string;
  title: string;
  summary?: string | null;
  sourceMetadata?: Record<string, unknown>;
  confidence?: number | null;
}

export interface DomainEvalFixtureWorkflow {
  kind?: string;
  scriptSource?: string;
  executionMode?: string;
}

export interface DomainEvalFixtureQuality {
  run?: boolean;
  sourceMetadata?: Record<string, unknown>;
  explicitUserApproval?: boolean;
}

export interface DomainEvalFixtureExecution {
  prompt?: string | null;
  agentId?: string | null;
  displayText?: string | null;
  providers?: Record<string, unknown>[];
  modelChain?: CodingEvalActiveModel[];
  compactConfig?: Record<string, unknown> | null;
  reasoningEffort?: string | null;
  extraSystemContext?: string | null;
  deniedTools?: string[];
  autoApproveTools?: boolean;
  workflowMode?: "off" | "on" | "ultracode" | string;
}

export interface DomainEvalFixtureChecks {
  expectedStatus?: string | null;
  minScore?: number | null;
  expectedPassedChecks?: string[];
  expectedFailedChecks?: string[];
  expectedExecutionStatus?: string | null;
  requireTurn?: boolean | null;
  minToolCalls?: number | null;
  expectedToolCalls?: string[];
  responseContains?: string[];
  errorContains?: string[];
}

export interface DomainEvalFixtureReport {
  fixtureRunId?: string | null;
  name: string;
  executionMode: string;
  sourceType: string;
  status: string;
  passed: boolean;
  sessionId: string;
  goalId?: string | null;
  workflowRunId?: string | null;
  qualityRunId?: string | null;
  evalRun?: DomainEvalRunRecord | null;
  execution?: DomainEvalFixtureExecutionReport | null;
  checks: DomainEvalFixtureCheck[];
  error?: string | null;
}

export interface DomainEvalFixtureExecutionReport {
  mode: string;
  status: string;
  prompt: string;
  agentId: string;
  workflowMode: string;
  turnId?: string | null;
  response?: string | null;
  error?: string | null;
  modelUsed?: CodingEvalActiveModel | null;
  toolCalls: string[];
}

export interface DomainEvalFixtureCheck {
  name: string;
  status: string;
  expected: string;
  actual: string;
  detail: string;
}

export interface ListDomainEvalFixtureRunsInput {
  sourceType?: string | null;
  executionMode?: string | null;
  status?: string | null;
  windowDays?: number | null;
  limit?: number | null;
}

export interface DomainEvalFixtureRunRecord {
  id: string;
  name: string;
  executionMode: string;
  sourceType: string;
  status: string;
  passed: boolean;
  sessionId: string;
  goalId?: string | null;
  workflowRunId?: string | null;
  qualityRunId?: string | null;
  evalRunId?: string | null;
  report: DomainEvalFixtureReport;
  error?: string | null;
  createdAt: string;
  updatedAt: string;
}

export interface DomainEvalCampaignModel {
  providerId?: string | null;
  modelId?: string | null;
  label?: string | null;
}

export interface CreateDomainEvalCampaignInput {
  sessionId?: string | null;
  projectId?: string | null;
  name?: string | null;
  domain?: string | null;
  taskIds?: string[];
  maxTasks?: number | null;
  models?: DomainEvalCampaignModel[];
  providers?: Record<string, unknown>[];
  executionMode?: string | null;
  runNow?: boolean;
  maxBudgetUsd?: number | null;
  timeoutSecs?: number | null;
}

export interface ListDomainEvalCampaignsInput {
  sessionId?: string | null;
  projectId?: string | null;
  limit?: number | null;
}

export interface DomainEvalCampaignLeaderboardInput {
  sessionId?: string | null;
  projectId?: string | null;
  domain?: string | null;
  windowDays?: number | null;
  limit?: number | null;
  campaignIds?: string[];
}

export interface RunDomainEvalCampaignInput {
  campaignId: string;
  providers?: Record<string, unknown>[];
  retryFailedOnly?: boolean;
}

export interface DomainEvalCampaignSummary {
  totalItems: number;
  queuedItems: number;
  runningItems: number;
  passedItems: number;
  failedItems: number;
  cancelledItems: number;
  interruptedItems: number;
  itemPassRate?: number | null;
  evalRuns: number;
  passedEvalRuns: number;
  failedEvalRuns: number;
  insufficientEvalRuns: number;
  averageScore?: number | null;
  totalChecks: number;
  passedChecks: number;
  failedChecks: number;
}

export interface DomainEvalCampaignItem {
  id: string;
  campaignId: string;
  taskId: string;
  taskTitle: string;
  domain: string;
  executionMode: string;
  providerId?: string | null;
  modelId?: string | null;
  label?: string | null;
  status: string;
  attempt: number;
  fixtureRunId?: string | null;
  evalRunId?: string | null;
  score?: number | null;
  totalChecks: number;
  passedChecks: number;
  failedChecks: number;
  startedAt?: string | null;
  finishedAt?: string | null;
  error?: string | null;
}

export interface DomainEvalCampaign {
  id: string;
  sessionId?: string | null;
  projectId?: string | null;
  name: string;
  status: string;
  domain?: string | null;
  taskFilter: Record<string, unknown>;
  modelMatrix: DomainEvalCampaignModel[];
  executionMode: string;
  maxBudgetUsd?: number | null;
  timeoutSecs?: number | null;
  summary: DomainEvalCampaignSummary;
  items: DomainEvalCampaignItem[];
  createdAt: string;
  updatedAt: string;
  startedAt?: string | null;
  finishedAt?: string | null;
  error?: string | null;
}

export interface DomainEvalCampaignLeaderboardEvidence {
  campaignId: string;
  campaignName: string;
  itemId: string;
  taskId: string;
  domain: string;
  executionMode: string;
  providerId?: string | null;
  modelId?: string | null;
  label?: string | null;
  status: string;
  score?: number | null;
  updatedAt: string;
  error?: string | null;
}

export interface DomainEvalCampaignLeaderboardRow {
  rank: number;
  label: string;
  providerId?: string | null;
  modelId?: string | null;
  executionMode: string;
  campaigns: number;
  items: number;
  passedItems: number;
  failedItems: number;
  cancelledItems: number;
  interruptedItems: number;
  attempts: number;
  evalRuns: number;
  itemPassRate?: number | null;
  averageScore?: number | null;
  totalChecks: number;
  failedChecks: number;
  domains: string[];
  warnings: string[];
  evidence: DomainEvalCampaignLeaderboardEvidence[];
}

export interface DomainEvalCampaignLeaderboardReport {
  generatedAt: string;
  status: string;
  scope: string;
  sessionId?: string | null;
  projectId?: string | null;
  domain?: string | null;
  windowDays: number;
  rows: DomainEvalCampaignLeaderboardRow[];
}

export interface ImportDomainEvalCaseInput {
  proposalId: string;
  overwrite?: boolean;
}

export interface ImportDomainEvalCaseResult {
  imported: boolean;
  task: DomainEvalTask;
  projectId?: string | null;
  sourcePath: string;
  importedAt: string;
}

export interface ListDomainEvalRunsInput {
  sessionId?: string | null;
  projectId?: string | null;
  domain?: string | null;
  taskId?: string | null;
  sourceType?: string | null;
  includeSynthetic?: boolean;
  windowDays?: number | null;
  limit?: number | null;
}

export interface DomainEvalSummary {
  requiredEvidence: number;
  satisfiedRequiredEvidence: number;
  missingRequiredEvidence: number;
  totalEvidence: number;
  sourceCount: number;
  datedSourceCount: number;
  dataQualityCount: number;
  userDecisionCount: number;
  workflowRuns: number;
  qualityState: string;
}

export interface DomainEvalCheck {
  name: string;
  category: string;
  status: "passed" | "failed" | "insufficient_data" | string;
  weight: number;
  score: number;
  expected: string;
  actual: string;
  detail: string;
}

export interface DomainEvalReport {
  task: DomainEvalTask;
  status: "passed" | "failed" | "insufficient_data" | string;
  score: number;
  summary: DomainEvalSummary;
  checks: DomainEvalCheck[];
  evidence: Record<string, unknown>;
  goal: Record<string, unknown>;
  quality: Record<string, unknown>;
  workflow: Record<string, unknown>;
}

export interface DomainEvalRunRecord {
  id: string;
  sessionId: string;
  projectId?: string | null;
  taskId: string;
  taskVersion: string;
  domain: string;
  label: string;
  status: "passed" | "failed" | "insufficient_data" | string;
  score: number;
  sourceType: string;
  report: DomainEvalReport;
  sourceQualityRunId?: string | null;
  createdAt: string;
}

export interface DomainQualityGateInput {
  sessionId?: string | null;
  projectId?: string | null;
  domain?: string | null;
  windowDays?: number | null;
  minEvalRuns?: number | null;
  minPassRate?: number | null;
  minAverageScore?: number | null;
  minQualityRuns?: number | null;
  maxBlockedQualityRuns?: number | null;
  minDomainCoverage?: number | null;
  requireApprovalSafety?: boolean;
  includeSynthetic?: boolean;
}

export interface DomainQualityGateThresholds {
  minEvalRuns: number;
  minPassRate: number;
  minAverageScore: number;
  minQualityRuns: number;
  maxBlockedQualityRuns: number;
  minDomainCoverage: number;
  requireApprovalSafety: boolean;
}

export interface DomainQualityGateSummary {
  evalRuns: number;
  passedEvalRuns: number;
  failedEvalRuns: number;
  insufficientEvalRuns: number;
  passRate?: number | null;
  averageScore?: number | null;
  qualityRuns: number;
  completedQualityRuns: number;
  blockedQualityRuns: number;
  failedQualityRuns: number;
  needsUserQualityRuns: number;
  approvalBlockers: number;
  domainsCovered: number;
  evidenceItems: number;
  sourceCited: number;
  datedSources: number;
  dataQualityChecked: number;
}

export interface DomainQualityGateCheck {
  name: string;
  status: "passed" | "failed" | "insufficient_data" | string;
  severity: string;
  expected: string;
  actual: string;
  detail: string;
}

export interface DomainQualityGateReport {
  generatedAt: string;
  status: "passed" | "failed" | "insufficient_data" | string;
  scope: "global" | "project" | "session" | string;
  sessionId?: string | null;
  projectId?: string | null;
  domain?: string | null;
  windowDays: number;
  since: string;
  thresholds: DomainQualityGateThresholds;
  summary: DomainQualityGateSummary;
  checks: DomainQualityGateCheck[];
}

export interface DomainReadinessGateInput {
  sessionId?: string | null;
  projectId?: string | null;
  domain?: string | null;
  windowDays?: number | null;
  minEvalRuns?: number | null;
  minPassRate?: number | null;
  minAverageScore?: number | null;
  minQualityRuns?: number | null;
  maxBlockedQualityRuns?: number | null;
  minDomainCoverage?: number | null;
  minCampaignItems?: number | null;
  minLeaderboardRows?: number | null;
  maxFailedCampaignItems?: number | null;
  maxOpenLearningProposals?: number | null;
  requireApprovalSafety?: boolean;
  includeSynthetic?: boolean;
}

export interface DomainReadinessGateThresholds {
  windowDays: number;
  minEvalRuns: number;
  minPassRate: number;
  minAverageScore: number;
  minQualityRuns: number;
  maxBlockedQualityRuns: number;
  minDomainCoverage: number;
  minCampaignItems: number;
  minLeaderboardRows: number;
  maxFailedCampaignItems: number;
  maxOpenLearningProposals: number;
  requireApprovalSafety: boolean;
  includeSynthetic: boolean;
}

export interface DomainReadinessGateSummary {
  evalRuns: number;
  qualityRuns: number;
  campaigns: number;
  activeCampaigns: number;
  terminalCampaigns: number;
  campaignItems: number;
  terminalCampaignItems: number;
  passedCampaignItems: number;
  failedCampaignItems: number;
  cancelledCampaignItems: number;
  interruptedCampaignItems: number;
  leaderboardRows: number;
  openLearningProposals: number;
  pendingLearningCampaigns: number;
  latestCampaignAt?: string | null;
  qualityStatus: string;
  leaderboardStatus: string;
}

export interface DomainReadinessGateCheck {
  name: string;
  status: "passed" | "failed" | "insufficient_data" | string;
  severity: string;
  expected: string;
  actual: string;
  detail: string;
}

export interface DomainReadinessGateReport {
  generatedAt: string;
  status: "passed" | "failed" | "insufficient_data" | string;
  scope: "global" | "project" | "session" | string;
  sessionId?: string | null;
  projectId?: string | null;
  domain?: string | null;
  since: string;
  thresholds: DomainReadinessGateThresholds;
  summary: DomainReadinessGateSummary;
  checks: DomainReadinessGateCheck[];
  qualityGate: DomainQualityGateReport;
  campaignLeaderboard: DomainEvalCampaignLeaderboardReport;
  blockers: string[];
  recommendedNextSteps: string[];
}

export interface DomainOperationalGateInput {
  sessionId?: string | null;
  projectId?: string | null;
  domain?: string | null;
  windowDays?: number | null;
  minWorkflowRuns?: number | null;
  maxFailedWorkflowRuns?: number | null;
  maxBlockedWorkflowRuns?: number | null;
  maxCancelledWorkflowRuns?: number | null;
  maxActiveWorkflowRuns?: number | null;
  minLoopRuns?: number | null;
  maxFailedLoopRuns?: number | null;
  maxActiveCampaigns?: number | null;
  maxFailedCampaignItems?: number | null;
}

export interface DomainOperationalGateThresholds {
  windowDays: number;
  minWorkflowRuns: number;
  maxFailedWorkflowRuns: number;
  maxBlockedWorkflowRuns: number;
  maxCancelledWorkflowRuns: number;
  maxActiveWorkflowRuns: number;
  minLoopRuns: number;
  maxFailedLoopRuns: number;
  maxActiveCampaigns: number;
  maxFailedCampaignItems: number;
}

export interface DomainOperationalGateSummary {
  workflowRuns: number;
  completedWorkflowRuns: number;
  failedWorkflowRuns: number;
  blockedWorkflowRuns: number;
  cancelledWorkflowRuns: number;
  activeWorkflowRuns: number;
  pausedWorkflowRuns: number;
  awaitingApprovalWorkflowRuns: number;
  loopSchedules: number;
  activeLoopSchedules: number;
  loopRuns: number;
  succeededLoopRuns: number;
  failedLoopRuns: number;
  activeLoopRuns: number;
  campaigns: number;
  activeCampaigns: number;
  campaignItems: number;
  passedCampaignItems: number;
  failedCampaignItems: number;
  cancelledCampaignItems: number;
  interruptedCampaignItems: number;
  latestActivityAt?: string | null;
  maxActiveWorkAgeSecs?: number | null;
}

export interface DomainOperationalGateCheck {
  name: string;
  status: "passed" | "failed" | "insufficient_data" | string;
  severity: string;
  expected: string;
  actual: string;
  detail: string;
}

export interface DomainOperationalGateReport {
  generatedAt: string;
  status: "passed" | "failed" | "insufficient_data" | string;
  scope: "global" | "project" | "session" | string;
  sessionId?: string | null;
  projectId?: string | null;
  domain?: string | null;
  since: string;
  thresholds: DomainOperationalGateThresholds;
  summary: DomainOperationalGateSummary;
  checks: DomainOperationalGateCheck[];
  blockers: string[];
  recommendedNextSteps: string[];
}

export interface DomainSoakReportInput {
  sessionId?: string | null;
  projectId?: string | null;
  domain?: string | null;
  windowDays?: number | null;
  maxItems?: number | null;
}

export interface DomainSoakReportSummary {
  workflowRuns: number;
  completedWorkflowRuns: number;
  failedWorkflowRuns: number;
  blockedWorkflowRuns: number;
  cancelledWorkflowRuns: number;
  activeWorkflowRuns: number;
  awaitingApprovalWorkflowRuns: number;
  repairWorkflowRuns: number;
  approvalEvents: number;
  approvalRequestEvents: number;
  approvalDecisionEvents: number;
  openApprovalWaits: number;
  pauseEvents: number;
  resumeEvents: number;
  cancelEvents: number;
  recoveryEvents: number;
  workflowControlInterventionEvents: number;
  workflowBudgetUsageEvents: number;
  workflowBudgetExhaustedEvents: number;
  maxWorkflowOutputTokensSpent?: number | null;
  maxWorkflowOutputTokenBudget?: number | null;
  averageApprovalWaitSecs?: number | null;
  maxApprovalWaitSecs?: number | null;
  maxOpenApprovalWaitSecs?: number | null;
  averageWorkflowDrainSecs?: number | null;
  maxWorkflowDrainSecs?: number | null;
  loopRuns: number;
  succeededLoopRuns: number;
  failedLoopRuns: number;
  activeLoopRuns: number;
  averageLoopDurationSecs?: number | null;
  maxLoopDurationSecs?: number | null;
  campaigns: number;
  activeCampaigns: number;
  campaignItems: number;
  passedCampaignItems: number;
  failedCampaignItems: number;
  cancelledCampaignItems: number;
  interruptedCampaignItems: number;
  retriedCampaignItems: number;
  averageCampaignItemDurationSecs?: number | null;
  maxCampaignItemDurationSecs?: number | null;
  connectorE2eEvidence: number;
  connectorExecutionEvidence: number;
  connectorVerificationEvidence: number;
  incidents: number;
  criticalIncidents: number;
  warningIncidents: number;
  totalRecords: number;
}

export interface DomainSoakIncident {
  source: string;
  id: string;
  title: string;
  status: string;
  severity: "critical" | "warning" | string;
  startedAt?: string | null;
  finishedAt?: string | null;
  durationSecs?: number | null;
  reason: string;
  recommendation: string;
}

export interface DomainSoakTimelineItem {
  source: string;
  id: string;
  label: string;
  status: string;
  at: string;
  durationSecs?: number | null;
}

export interface DomainSoakReport {
  generatedAt: string;
  status: "passed" | "failed" | "insufficient_data" | string;
  scope: "global" | "project" | "session" | string;
  sessionId?: string | null;
  projectId?: string | null;
  domain?: string | null;
  windowDays: number;
  since: string;
  until: string;
  summary: DomainSoakReportSummary;
  incidents: DomainSoakIncident[];
  timeline: DomainSoakTimelineItem[];
  recommendedNextSteps: string[];
  markdown: string;
  operationalGate: DomainOperationalGateReport;
}

export interface CodingTrendOverview {
  sessions: number;
  goals: number;
  completedGoals: number;
  blockedGoals: number;
  workflowRuns: number;
  completedWorkflows: number;
  blockedWorkflows: number;
  failedWorkflows: number;
  goalCompletionRate?: number | null;
  workflowCompletionRate?: number | null;
}

export interface CodingEvalTrend {
  runs: number;
  passed: number;
  failed: number;
  successRate?: number | null;
  backlogCandidates: number;
}

export interface CodingReviewTrend {
  runs: number;
  findings: number;
  blockingFindings: number;
  resolvedFindings: number;
  falsePositiveFindings: number;
  byCategory: CodingMetricBucket[];
}

export interface CodingVerificationTrend {
  runs: number;
  steps: number;
  passedSteps: number;
  failedSteps: number;
  timedOutSteps: number;
  plannedOnlyRuns: number;
  executedSuccessRate?: number | null;
  recommendationCoverage?: number | null;
}

export interface CodingRepairLoopTrend {
  runs: number;
  completed: number;
  blocked: number;
  exhausted: number;
  successRate?: number | null;
}

export interface CodingRetroTrend {
  total: number;
  completed: number;
  blocked: number;
  failed: number;
  cancelled: number;
  recommendations: number;
  latestSummary?: string | null;
}

export interface CodingMetricBucket {
  key: string;
  label: string;
  count: number;
}

export interface CodingFailureBucket {
  category: string;
  label: string;
  count: number;
  severity: string;
  examples: string[];
}

export interface CodingRunSummary {
  runId: string;
  sessionId: string;
  goalId?: string | null;
  kind: string;
  state: string;
  blockedReason?: string | null;
  failureCategory?: string | null;
  updatedAt: string;
}

export interface CodingRetroSignal {
  kind: string;
  label: string;
  severity: string;
  detail?: string | null;
}

export interface CodingRetroRecommendation {
  kind: string;
  title: string;
  rationale: string;
}

export interface CodingWorkflowRetro {
  id: string;
  sessionId: string;
  projectId?: string | null;
  workflowRunId: string;
  runState: string;
  summary: string;
  signals: CodingRetroSignal[];
  recommendations: CodingRetroRecommendation[];
  createdAt: string;
  updatedAt: string;
}

export interface CodingImprovementProposal {
  id: string;
  sessionId: string;
  projectId?: string | null;
  kind: string;
  status: string;
  sourceType: string;
  sourceId: string;
  title: string;
  body: string;
  payload: Record<string, unknown>;
  fingerprint: string;
  action?: CodingImprovementActionRecord | null;
  promotion?: CodingImprovementPromotionRecord | null;
  createdAt: string;
  updatedAt: string;
  decidedAt?: string | null;
}

export interface CodingImprovementActionRecord {
  applied: boolean;
  artifacts: CodingImprovementActionArtifact[];
  error?: string | null;
  appliedAt?: string | null;
}

export interface CodingImprovementActionArtifact {
  kind: string;
  path: string;
  contentHash?: string | null;
}

export interface CodingImprovementActionStep {
  action: string;
  label: string;
  targetPath: string;
  targetExists: boolean;
  contentPreview?: string | null;
}

export interface CodingImprovementActionPlan {
  proposal: CodingImprovementProposal;
  targetKind: string;
  summary: string;
  requiresConfirmation: boolean;
  steps: CodingImprovementActionStep[];
  preview: Record<string, unknown>;
}

export interface ApplyCodingImprovementProposalResult {
  proposal: CodingImprovementProposal;
  plan: CodingImprovementActionPlan;
  applied: boolean;
  artifacts: CodingImprovementActionArtifact[];
  error?: string | null;
}

export interface CodingImprovementPromotionRecord {
  promoted: boolean;
  artifacts: CodingImprovementActionArtifact[];
  error?: string | null;
  promotedAt?: string | null;
}

export interface CodingImprovementPromotionStep {
  action: string;
  label: string;
  sourcePath?: string | null;
  targetPath: string;
  targetExists: boolean;
  sourceHash?: string | null;
  contentPreview?: string | null;
}

export interface CodingImprovementPromotionPlan {
  proposal: CodingImprovementProposal;
  targetKind: string;
  summary: string;
  requiresConfirmation: boolean;
  steps: CodingImprovementPromotionStep[];
  preview: Record<string, unknown>;
}

export interface PromoteCodingImprovementProposalResult {
  proposal: CodingImprovementProposal;
  plan: CodingImprovementPromotionPlan;
  promoted: boolean;
  artifacts: CodingImprovementActionArtifact[];
  error?: string | null;
}

export interface CodingTrendReport {
  sessionId: string;
  projectId?: string | null;
  scope: string;
  windowDays: number;
  generatedAt: string;
  overview: CodingTrendOverview;
  eval: CodingEvalTrend;
  review: CodingReviewTrend;
  verification: CodingVerificationTrend;
  repairLoop: CodingRepairLoopTrend;
  retro: CodingRetroTrend;
  failures: CodingFailureBucket[];
  recentRuns: CodingRunSummary[];
  retros: CodingWorkflowRetro[];
  proposals: CodingImprovementProposal[];
}

export interface GenerateCodingImprovementProposalsResult {
  inserted: number;
  proposals: CodingImprovementProposal[];
}

export interface DistillCodingImprovementResult {
  inserted: number;
  distillation: CodingImprovementDistillation;
  proposals: CodingImprovementProposal[];
}

export interface CodingImprovementDistillation {
  sessionId: string;
  projectId?: string | null;
  scope: string;
  generatedAt: string;
  transcript: CodingTranscriptDistillation;
  workflowPatterns: CodingWorkflowPatternDistillation[];
  failureFeedback: CodingFailureFeedback[];
  candidates: CodingDistilledCandidate[];
}

export interface CodingTranscriptDistillation {
  sessionsScanned: number;
  messagesScanned: number;
  userMessages: number;
  assistantMessages: number;
  toolCalls: number;
  toolErrors: number;
  topTools: CodingToolUsageDistillation[];
  objectiveSnippets: string[];
  errorSnippets: string[];
}

export interface CodingToolUsageDistillation {
  toolName: string;
  calls: number;
  errors: number;
  avgDurationMs?: number | null;
}

export interface CodingWorkflowPatternDistillation {
  runId: string;
  sessionId: string;
  kind: string;
  state: string;
  executionMode: string;
  opCount: number;
  completedOps: number;
  failedOps: number;
  hasReview: boolean;
  hasVerification: boolean;
  hasDiff: boolean;
  toolOps: string[];
  summary: string;
}

export interface CodingFailureFeedback {
  category: string;
  label: string;
  severity: string;
  count: number;
  rule: string;
  expectedSignals: string[];
  examples: string[];
}

export interface CodingDistilledCandidate {
  kind: string;
  sourceType: string;
  sourceId: string;
  title: string;
  rationale: string;
  fingerprint: string;
}

export interface CodingEvalGoldTaskPackRunInput {
  sessionId?: string | null;
  projectId?: string | null;
  ids?: string[];
  statuses?: string[];
  taskTypes?: string[];
  includeUnautomated?: boolean;
  maxTasks?: number | null;
  executionMode?: "agent" | "fixture_patch" | string | null;
  providers?: Record<string, unknown>[];
  modelChain?: CodingEvalActiveModel[];
  compactConfig?: Record<string, unknown> | null;
  reasoningEffort?: string | null;
  extraSystemContext?: string | null;
  deniedTools?: string[];
  autoApproveTools?: boolean;
  recordEvalRuns?: boolean;
  recordPackRun?: boolean;
  evaluateGoal?: boolean;
  label?: string | null;
  baselineKind?: string | null;
  sourceType?: string | null;
  sourceId?: string | null;
}

export interface CodingEvalGoldTaskPackSummary {
  packId: string;
  sourceDoc: string;
  totalCases: number;
  automatedCases: number;
  activeCases: number;
  cases: CodingEvalGoldTaskCaseSummary[];
}

export interface CodingEvalGoldTaskCaseSummary {
  id: string;
  taskType: string;
  title: string;
  status: string;
  source: string;
  executionMode: string;
  automationStatus: string;
  fixtureName?: string | null;
  expectedArtifacts: string[];
  requiresSeededState: boolean;
  likelyFiles: string[];
  allowedValidation: string[];
  successCriteria: string[];
}

export interface CodingEvalGoldTaskPackReport {
  packId: string;
  sourceDoc: string;
  packRunId?: string | null;
  selectedCases: number;
  automatedCases: number;
  skippedCases: number;
  passedCases: number;
  failedCases: number;
  totalChecks: number;
  passed: boolean;
  cases: CodingEvalGoldTaskCaseRunReport[];
}

export interface CodingEvalGoldTaskCaseRunReport {
  case: CodingEvalGoldTaskCaseSummary;
  status: string;
  fixtureName?: string | null;
  report?: CodingEvalFixtureReport | null;
  error?: string | null;
}

export interface CodingEvalStrategyEffectInput {
  sessionId?: string | null;
  projectId?: string | null;
  baselinePackRunId?: string | null;
  candidatePackRunId?: string | null;
  recordRun?: boolean;
  sourceType?: string | null;
  sourceId?: string | null;
  strategyType?: string | null;
  baselineLabel?: string | null;
  candidateLabel?: string | null;
  baseline: CodingEvalGoldTaskPackReport;
  candidate: CodingEvalGoldTaskPackReport;
}

export interface CodingEvalStrategyEffectReport {
  runId?: string | null;
  strategyType: string;
  baselineLabel: string;
  candidateLabel: string;
  verdict: string;
  comparedCases: number;
  baselineOnlyCases: string[];
  candidateOnlyCases: string[];
  summary: CodingEvalStrategyEffectSummary;
  dimensions: CodingEvalStrategyEffectDimension[];
  cases: CodingEvalStrategyCaseComparison[];
  regressions: string[];
  improvements: string[];
}

export interface CodingEvalStrategyEffectSummary {
  baselinePassRate: number;
  candidatePassRate: number;
  passRateDelta: number;
  baselineAverageScore: number;
  candidateAverageScore: number;
  averageScoreDelta: number;
  baselineContextRecall: number;
  candidateContextRecall: number;
  contextRecallDelta: number;
  baselineValidationViolations: number;
  candidateValidationViolations: number;
  validationViolationDelta: number;
  baselineScopeCreep: number;
  candidateScopeCreep: number;
  scopeCreepDelta: number;
  baselineExecutionFailures: number;
  candidateExecutionFailures: number;
  executionFailureDelta: number;
}

export interface CodingEvalStrategyEffectDimension {
  name: string;
  direction: "higher" | "lower" | string;
  baseline: number;
  candidate: number;
  delta: number;
  verdict: string;
  detail: string;
}

export interface CodingEvalStrategyCaseComparison {
  id: string;
  title: string;
  verdict: string;
  baselineStatus: string;
  candidateStatus: string;
  baselinePassed: boolean;
  candidatePassed: boolean;
  baselineOutcome?: string | null;
  candidateOutcome?: string | null;
  baselineScore: number;
  candidateScore: number;
  scoreDelta: number;
  baselineContextRecall: number;
  candidateContextRecall: number;
  contextRecallDelta: number;
  baselineValidationViolations: number;
  candidateValidationViolations: number;
  baselineScopeCreep: number;
  candidateScopeCreep: number;
  baselineExecutionFailed: boolean;
  candidateExecutionFailed: boolean;
  notes: string[];
}

export interface CodingEvalReleaseGateInput {
  sessionId?: string | null;
  projectId?: string | null;
  windowDays?: number | null;
  minPackRuns?: number | null;
  minStrategyEffectRuns?: number | null;
  minPackPassRate?: number | null;
  requireExternalModelPack?: boolean;
  maxRegressedStrategyEffects?: number | null;
  maxMixedStrategyEffects?: number | null;
  maxMissingToolCallRuns?: number | null;
  maxValidationViolationDelta?: number | null;
  maxScopeCreepDelta?: number | null;
}

export interface CodingEvalReleaseGateThresholds {
  minPackRuns: number;
  minStrategyEffectRuns: number;
  minPackPassRate: number;
  requireExternalModelPack: boolean;
  maxRegressedStrategyEffects: number;
  maxMixedStrategyEffects: number;
  maxMissingToolCallRuns: number;
  maxValidationViolationDelta: number;
  maxScopeCreepDelta: number;
}

export interface CodingEvalReleaseGateSummary {
  packRuns: number;
  passedPackRuns: number;
  failedPackRuns: number;
  skippedPackRuns: number;
  packPassRate?: number | null;
  deterministicPackRuns: number;
  mockProviderPackRuns: number;
  externalModelPackRuns: number;
  passedCases: number;
  failedCases: number;
  skippedCases: number;
  totalChecks: number;
  strategyEffectRuns: number;
  improvedStrategyEffects: number;
  regressedStrategyEffects: number;
  mixedStrategyEffects: number;
  inconclusiveStrategyEffects: number;
  validationViolationDelta: number;
  scopeCreepDelta: number;
  executionFailureDelta: number;
  missingToolCallRuns: number;
}

export interface CodingEvalReleaseGateCheck {
  name: string;
  status: "passed" | "failed" | "insufficient_data" | string;
  severity: string;
  expected: string;
  actual: string;
  detail: string;
}

export interface CodingEvalReleaseGateReport {
  generatedAt: string;
  status: "passed" | "failed" | "insufficient_data" | string;
  scope: "global" | "project" | "session" | string;
  sessionId?: string | null;
  projectId?: string | null;
  windowDays: number;
  since: string;
  thresholds: CodingEvalReleaseGateThresholds;
  summary: CodingEvalReleaseGateSummary;
  checks: CodingEvalReleaseGateCheck[];
}

export interface CodingLearningGeneralizationInput {
  sessionId?: string | null;
  projectId?: string | null;
  windowDays?: number | null;
  sourceType?: string | null;
  sourceId?: string | null;
  proposalKinds?: string[];
  minProjects?: number | null;
  minProjectPackRuns?: number | null;
  minProjectPackPassRate?: number | null;
  minStrategyEffectRunsPerProject?: number | null;
  requirePromotedLearning?: boolean;
  requireExternalModelPack?: boolean;
  maxRegressedProjects?: number | null;
  maxMixedProjects?: number | null;
  maxValidationViolationDeltaPerProject?: number | null;
  maxScopeCreepDeltaPerProject?: number | null;
}

export interface CodingLearningGeneralizationThresholds {
  minProjects: number;
  minProjectPackRuns: number;
  minProjectPackPassRate: number;
  minStrategyEffectRunsPerProject: number;
  requirePromotedLearning: boolean;
  requireExternalModelPack: boolean;
  maxRegressedProjects: number;
  maxMixedProjects: number;
  maxValidationViolationDeltaPerProject: number;
  maxScopeCreepDeltaPerProject: number;
}

export interface CodingLearningGeneralizationSummary {
  projectsEvaluated: number;
  projectsWithPromotedLearning: number;
  projectsWithPackRuns: number;
  projectsWithStrategyEffects: number;
  projectsWithExternalModelPack: number;
  passedProjects: number;
  failedProjects: number;
  insufficientProjects: number;
  totalPromotedLearning: number;
  totalPackRuns: number;
  totalStrategyEffectRuns: number;
  regressedProjects: number;
  mixedProjects: number;
}

export interface CodingLearningGeneralizationItem {
  proposalId: string;
  projectId: string;
  kind: string;
  title: string;
  sourceType: string;
  sourceId: string;
  promotedAt: string;
}

export interface CodingLearningGeneralizationProject {
  projectId: string;
  status: "passed" | "failed" | "insufficient_data" | string;
  promotedLearning: number;
  packRuns: number;
  passedPackRuns: number;
  failedPackRuns: number;
  packPassRate?: number | null;
  externalModelPackRuns: number;
  strategyEffectRuns: number;
  improvedStrategyEffects: number;
  regressedStrategyEffects: number;
  mixedStrategyEffects: number;
  validationViolationDelta: number;
  scopeCreepDelta: number;
  executionFailureDelta: number;
  reasons: string[];
  learningItems: CodingLearningGeneralizationItem[];
}

export interface CodingLearningGeneralizationCheck {
  name: string;
  status: "passed" | "failed" | "insufficient_data" | string;
  severity: string;
  expected: string;
  actual: string;
  detail: string;
}

export interface CodingLearningGeneralizationReport {
  generatedAt: string;
  status: "passed" | "failed" | "insufficient_data" | string;
  scope: "global" | "project" | "session" | string;
  sessionId?: string | null;
  projectId?: string | null;
  windowDays: number;
  since: string;
  sourceType?: string | null;
  sourceId?: string | null;
  proposalKinds: string[];
  thresholds: CodingLearningGeneralizationThresholds;
  summary: CodingLearningGeneralizationSummary;
  projects: CodingLearningGeneralizationProject[];
  checks: CodingLearningGeneralizationCheck[];
}

export interface CodingBenchmarkCenterInput {
  sessionId?: string | null;
  projectId?: string | null;
  windowDays?: number | null;
  limit?: number | null;
  requireExternalModelBaseline?: boolean;
  requireLearningGeneralization?: boolean;
}

export interface CodingBenchmarkCenterSummary {
  totalRuns: number;
  passedRuns: number;
  failedRuns: number;
  skippedRuns: number;
  deterministicRuns: number;
  externalModelRuns: number;
  selectedCases: number;
  automatedCases: number;
  passedCases: number;
  failedCases: number;
  skippedCases: number;
  totalChecks: number;
  runPassRate?: number | null;
  casePassRate?: number | null;
  bestCasePassRate?: number | null;
  latestRunId?: string | null;
  latestRunStatus?: string | null;
  latestRunAt?: string | null;
}

export interface CodingBenchmarkRunItem {
  id: string;
  sessionId?: string | null;
  projectId?: string | null;
  packId: string;
  sourceDoc: string;
  label?: string | null;
  baselineKind: string;
  status: "passed" | "failed" | "skipped" | string;
  selectedCases: number;
  automatedCases: number;
  skippedCases: number;
  passedCases: number;
  failedCases: number;
  totalChecks: number;
  casePassRate?: number | null;
  sourceType?: string | null;
  sourceId?: string | null;
  createdAt: string;
  failedCasesSummary: string[];
}

export interface CodingBenchmarkBaselineBucket {
  baselineKind: string;
  runs: number;
  passedRuns: number;
  failedRuns: number;
  skippedRuns: number;
  passedCases: number;
  failedCases: number;
  runPassRate?: number | null;
  casePassRate?: number | null;
  latestRunAt?: string | null;
}

export interface CodingBenchmarkCenterCheck {
  name: string;
  status: "passed" | "failed" | "insufficient_data" | string;
  severity: "required" | "advisory" | string;
  expected: string;
  actual: string;
  detail: string;
}

export interface CodingBenchmarkCenterReport {
  generatedAt: string;
  status: "passed" | "failed" | "insufficient_data" | string;
  scope: "global" | "project" | "session" | string;
  sessionId?: string | null;
  projectId?: string | null;
  windowDays: number;
  since: string;
  summary: CodingBenchmarkCenterSummary;
  baselines: CodingBenchmarkBaselineBucket[];
  runs: CodingBenchmarkRunItem[];
  checks: CodingBenchmarkCenterCheck[];
  releaseGate: CodingEvalReleaseGateReport;
  generalizationGate: CodingLearningGeneralizationReport;
}

export interface CodingBenchmarkCampaignModel {
  providerId?: string | null;
  modelId?: string | null;
  label?: string | null;
}

export interface CodingBenchmarkCampaignCreateInput {
  sessionId?: string | null;
  projectId?: string | null;
  name?: string | null;
  goldTaskInput?: CodingEvalGoldTaskPackRunInput;
  models?: CodingBenchmarkCampaignModel[];
  runNow?: boolean;
  maxBudgetUsd?: number | null;
  timeoutSecs?: number | null;
}

export interface CodingBenchmarkCampaignListInput {
  sessionId?: string | null;
  projectId?: string | null;
  limit?: number | null;
}

export interface CodingBenchmarkCampaignRunInput {
  campaignId: string;
  providers?: Record<string, unknown>[];
  retryFailedOnly?: boolean;
}

export interface CodingBenchmarkCampaignSummary {
  totalItems: number;
  queuedItems: number;
  runningItems: number;
  passedItems: number;
  failedItems: number;
  skippedItems: number;
  cancelledItems: number;
  interruptedItems: number;
  itemPassRate?: number | null;
  selectedCases: number;
  passedCases: number;
  failedCases: number;
  skippedCases: number;
  totalChecks: number;
  casePassRate?: number | null;
}

export interface CodingBenchmarkCampaignItem {
  id: string;
  campaignId: string;
  providerId?: string | null;
  modelId?: string | null;
  label?: string | null;
  status: string;
  attempt: number;
  packRunId?: string | null;
  selectedCases: number;
  passedCases: number;
  failedCases: number;
  skippedCases: number;
  totalChecks: number;
  startedAt?: string | null;
  finishedAt?: string | null;
  error?: string | null;
}

export interface CodingBenchmarkCampaign {
  id: string;
  sessionId?: string | null;
  projectId?: string | null;
  name: string;
  status: string;
  taskPackId: string;
  sourceDoc: string;
  executionMode: string;
  baselineKind: string;
  taskFilter: Record<string, unknown>;
  modelMatrix: CodingBenchmarkCampaignModel[];
  maxBudgetUsd?: number | null;
  timeoutSecs?: number | null;
  summary: CodingBenchmarkCampaignSummary;
  items: CodingBenchmarkCampaignItem[];
  createdAt: string;
  updatedAt: string;
  startedAt?: string | null;
  finishedAt?: string | null;
  error?: string | null;
}

export interface CodingBenchmarkLeaderboardInput {
  sessionId?: string | null;
  projectId?: string | null;
  windowDays?: number | null;
  campaignIds?: string[];
  limit?: number | null;
  minItems?: number | null;
}

export type CodingBenchmarkComparisonInput = CodingBenchmarkLeaderboardInput;

export interface CodingBenchmarkLeaderboardEvidence {
  campaignId: string;
  campaignName: string;
  itemId: string;
  packRunId?: string | null;
  providerId?: string | null;
  modelId?: string | null;
  label?: string | null;
  status: string;
  updatedAt: string;
  error?: string | null;
}

export interface CodingBenchmarkLeaderboardRow {
  rank: number;
  label: string;
  providerId?: string | null;
  modelId?: string | null;
  taskPackId: string;
  sourceDoc: string;
  executionMode: string;
  baselineKind: string;
  campaigns: number;
  items: number;
  passedItems: number;
  failedItems: number;
  skippedItems: number;
  cancelledItems: number;
  interruptedItems: number;
  attempts: number;
  selectedCases: number;
  passedCases: number;
  failedCases: number;
  skippedCases: number;
  totalChecks: number;
  itemPassRate?: number | null;
  casePassRate?: number | null;
  warnings: string[];
  evidence: CodingBenchmarkLeaderboardEvidence[];
}

export interface CodingBenchmarkLeaderboardReport {
  generatedAt: string;
  status: string;
  scope: string;
  sessionId?: string | null;
  projectId?: string | null;
  windowDays: number;
  since: string;
  minItems: number;
  rows: CodingBenchmarkLeaderboardRow[];
  checks: CodingBenchmarkCenterCheck[];
}

export interface CodingBenchmarkTaskPackTaskManifest {
  taskId: string;
  version: string;
  title: string;
  status?: string | null;
  taskType: string;
  difficulty: string;
  language?: string | null;
  framework?: string | null;
  sourceUri?: string | null;
  repoTemplate?: string | null;
  tags?: string[];
  successCriteria?: string[];
  validationCommands?: string[];
  allowedPaths?: string[];
  forbiddenPaths?: string[];
  calibrationNotes?: string[];
  calibratedAt?: string | null;
  licenseNote?: string | null;
  privacyNote?: string | null;
  redactionStatus?: string | null;
}

export interface CodingBenchmarkTaskPackManifest {
  packId: string;
  version: string;
  name: string;
  description?: string | null;
  status?: string | null;
  sourceKind: string;
  sourceUri?: string | null;
  repoTemplate?: string | null;
  licenseNote: string;
  privacyNote: string;
  redactionStatus: string;
  tasks: CodingBenchmarkTaskPackTaskManifest[];
}

export interface CodingBenchmarkTaskPackImportInput {
  manifest: CodingBenchmarkTaskPackManifest;
  explicitImportConsent: boolean;
  importedFrom?: string | null;
}

export interface CodingBenchmarkTaskPackListInput {
  status?: string | null;
  includeArchived?: boolean;
  limit?: number | null;
}

export interface CodingBenchmarkTaskPackStatusInput {
  packId: string;
  version: string;
  status: string;
}

export interface CodingBenchmarkTaskPackValidateInput {
  packId: string;
  version: string;
}

export interface CodingBenchmarkCorpusHealthInput {
  staleAfterDays?: number | null;
}

export interface CodingBenchmarkTaskPackTask {
  id: string;
  packId: string;
  packVersion: string;
  taskId: string;
  version: string;
  title: string;
  status: string;
  taskType: string;
  difficulty: string;
  language?: string | null;
  framework?: string | null;
  sourceUri?: string | null;
  repoTemplate?: string | null;
  tags: string[];
  successCriteria: string[];
  validationCommands: string[];
  allowedPaths: string[];
  forbiddenPaths: string[];
  calibrationNotes: string[];
  calibratedAt?: string | null;
  licenseNote?: string | null;
  privacyNote?: string | null;
  redactionStatus: string;
  riskFlags: string[];
  fingerprint: string;
  createdAt: string;
  updatedAt: string;
}

export interface CodingBenchmarkTaskPack {
  id: string;
  packId: string;
  version: string;
  name: string;
  description?: string | null;
  status: string;
  sourceKind: string;
  sourceUri?: string | null;
  repoTemplate?: string | null;
  licenseNote: string;
  privacyNote: string;
  redactionStatus: string;
  importedFrom?: string | null;
  tasks: CodingBenchmarkTaskPackTask[];
  createdAt: string;
  updatedAt: string;
  activatedAt?: string | null;
  archivedAt?: string | null;
}

export interface CodingBenchmarkTaskPackValidationReport {
  generatedAt: string;
  status: string;
  packId: string;
  version: string;
  checks: CodingBenchmarkCenterCheck[];
  warnings: string[];
}

export interface CodingBenchmarkCorpusDuplicate {
  fingerprint: string;
  tasks: string[];
}

export interface CodingBenchmarkCorpusHealthReport {
  generatedAt: string;
  status: string;
  staleAfterDays: number;
  packs: number;
  activePacks: number;
  draftPacks: number;
  archivedPacks: number;
  tasks: number;
  activeTasks: number;
  draftTasks: number;
  archivedTasks: number;
  byDifficulty: CodingMetricBucket[];
  byTaskType: CodingMetricBucket[];
  byLanguage: CodingMetricBucket[];
  staleTasks: string[];
  duplicateTasks: CodingBenchmarkCorpusDuplicate[];
  gamingRiskTasks: string[];
  checks: CodingBenchmarkCenterCheck[];
}

export interface CodingBenchmarkReportGenerateInput {
  reportType: string;
  title?: string | null;
  sessionId?: string | null;
  projectId?: string | null;
  campaignId?: string | null;
  campaignIds?: string[];
  windowDays?: number | null;
  markReleaseEvidence?: boolean;
  outputDir?: string | null;
}

export interface CodingBenchmarkReportListInput {
  sessionId?: string | null;
  projectId?: string | null;
  releaseEvidenceOnly?: boolean;
  limit?: number | null;
}

export interface CodingBenchmarkReportMarkInput {
  reportId: string;
  releaseEvidence: boolean;
}

export interface CodingBenchmarkReport {
  id: string;
  reportType: string;
  title: string;
  status: string;
  summary: string;
  scope: string;
  sessionId?: string | null;
  projectId?: string | null;
  sourceType: string;
  sourceId: string;
  campaignId?: string | null;
  campaignIds: string[];
  snapshot: Record<string, unknown>;
  markdownPath: string;
  jsonPath: string;
  htmlPath: string;
  releaseEvidence: boolean;
  createdAt: string;
  updatedAt: string;
  markedReleaseAt?: string | null;
}

export interface CodingContinuousBenchmarkGateInput {
  sessionId?: string | null;
  projectId?: string | null;
  triggerKind?: string | null;
  windowDays?: number | null;
  maxEvidenceAgeDays?: number | null;
  requireReleaseReportEvidence?: boolean;
  requireRecentCampaign?: boolean;
  requiredTaskPackId?: string | null;
  requiredBaselineKind?: string | null;
  requiredProviderId?: string | null;
  requiredModelId?: string | null;
  requireExternalModel?: boolean;
  externalModelPolicyEnabled?: boolean;
  minCampaignItems?: number | null;
  minCasePassRate?: number | null;
  maxOpenBacklogItems?: number | null;
  maxInterruptedCampaigns?: number | null;
  maxProviderErrorItems?: number | null;
  maxBudgetExhaustedItems?: number | null;
  maxBudgetUsd?: number | null;
}

export interface CodingContinuousBenchmarkGateThresholds {
  triggerKind: string;
  windowDays: number;
  maxEvidenceAgeDays: number;
  requireReleaseReportEvidence: boolean;
  requireRecentCampaign: boolean;
  requiredTaskPackId?: string | null;
  requiredBaselineKind?: string | null;
  requiredProviderId?: string | null;
  requiredModelId?: string | null;
  requireExternalModel: boolean;
  externalModelPolicyEnabled: boolean;
  minCampaignItems: number;
  minCasePassRate: number;
  maxOpenBacklogItems: number;
  maxInterruptedCampaigns: number;
  maxProviderErrorItems: number;
  maxBudgetExhaustedItems: number;
  maxBudgetUsd?: number | null;
}

export interface CodingContinuousBenchmarkReliability {
  campaigns: number;
  passedCampaigns: number;
  failedCampaigns: number;
  partialCampaigns: number;
  interruptedCampaigns: number;
  cancelledCampaigns: number;
  retryAttempts: number;
  retryPassedItems: number;
  providerErrorItems: number;
  budgetExhaustedItems: number;
  approvalWaitItems: number;
  campaignSuccessRate?: number | null;
  retrySuccessRate?: number | null;
  providerErrorRate?: number | null;
}

export interface CodingContinuousBenchmarkGateSummary {
  latestReleaseReportId?: string | null;
  latestReleaseEvidenceAt?: string | null;
  latestPassedAt?: string | null;
  freshReleaseEvidence: boolean;
  freshCampaigns: number;
  totalCampaignItems: number;
  passedCampaignItems: number;
  failedCampaignItems: number;
  interruptedCampaignItems: number;
  cancelledCampaignItems: number;
  selectedCases: number;
  passedCases: number;
  failedCases: number;
  casePassRate?: number | null;
  openBacklogItems: number;
  pendingFailureItems: number;
  maxCampaignBudgetUsd?: number | null;
  retentionDays: number;
  rawArtifactRetentionDays: number;
}

export interface CodingContinuousBenchmarkGateReport {
  generatedAt: string;
  status: string;
  scope: string;
  sessionId?: string | null;
  projectId?: string | null;
  since: string;
  staleBefore: string;
  thresholds: CodingContinuousBenchmarkGateThresholds;
  summary: CodingContinuousBenchmarkGateSummary;
  reliability: CodingContinuousBenchmarkReliability;
  checks: CodingBenchmarkCenterCheck[];
  releaseGate: CodingEvalReleaseGateReport;
  leaderboard: CodingBenchmarkLeaderboardReport;
  corpusHealth: CodingBenchmarkCorpusHealthReport;
  blockers: string[];
  recommendedNextSteps: string[];
}

export interface CodingBenchmarkBacklogListInput {
  sessionId?: string | null;
  projectId?: string | null;
  status?: string | null;
  limit?: number | null;
}

export interface CodingBenchmarkBacklogMaterializeInput {
  sessionId?: string | null;
  projectId?: string | null;
  campaignIds?: string[];
  windowDays?: number | null;
  limit?: number | null;
}

export interface CodingBenchmarkBacklogStatusInput {
  itemId: string;
  status: string;
  proposalId?: string | null;
}

export interface CodingBenchmarkBacklogItem {
  id: string;
  status: string;
  severity: string;
  title: string;
  failureCategory: string;
  scope: string;
  sessionId?: string | null;
  projectId?: string | null;
  campaignId: string;
  campaignItemId: string;
  packRunId?: string | null;
  taskPackId: string;
  taskId: string;
  providerId?: string | null;
  modelId?: string | null;
  label?: string | null;
  baselineKind: string;
  executionMode: string;
  evidence: Record<string, unknown>;
  proposalId?: string | null;
  createdAt: string;
  updatedAt: string;
  resolvedAt?: string | null;
}

export interface CodingBenchmarkBacklogMaterializeResult {
  inserted: number;
  existing: number;
  items: CodingBenchmarkBacklogItem[];
}

export interface DomainEvidenceRequirement {
  evidenceType: string;
  title: string;
  required: boolean;
  minCount?: number | null;
  metadataKeys: string[];
}

export interface DomainApprovalGate {
  action: string;
  reason: string;
  required: boolean;
}

export interface DomainVerificationRule {
  rule: string;
  severity: string;
  description: string;
}

export interface DomainWorkflowTemplate {
  id: string;
  version: string;
  title: string;
  domain: string;
  taskTypes: string[];
  defaultMode: string;
  requiredEvidence: DomainEvidenceRequirement[];
  recommendedTools: string[];
  approvalGates: DomainApprovalGate[];
  verificationPolicy: DomainVerificationRule[];
  stopConditions: string[];
  outputContract: string;
  evalCriteria: string[];
  promptHints: string[];
  scope: string;
  projectId?: string | null;
  enabled: boolean;
  createdAt: string;
  updatedAt: string;
}

export interface ListDomainWorkflowTemplatesInput {
  domain?: string | null;
  taskType?: string | null;
  projectId?: string | null;
  includeDisabled?: boolean;
  limit?: number | null;
}

export interface DomainWorkflowTemplateDraft {
  id: string;
  version?: string;
  title: string;
  domain: string;
  taskTypes?: string[];
  defaultMode?: string;
  requiredEvidence?: DomainEvidenceRequirement[];
  recommendedTools?: string[];
  approvalGates?: DomainApprovalGate[];
  verificationPolicy?: DomainVerificationRule[];
  stopConditions?: string[];
  outputContract?: string;
  evalCriteria?: string[];
  promptHints?: string[];
  scope?: string;
  projectId?: string | null;
  enabled?: boolean;
}

export interface SaveDomainWorkflowTemplateInput {
  template: DomainWorkflowTemplateDraft;
  explicitSaveConsent: boolean;
}

export interface PreviewDomainWorkflowInput {
  templateId: string;
  version?: string | null;
  sessionId: string;
  goalId?: string | null;
  taskType?: string | null;
  objective?: string | null;
  modeOverride?: string | null;
  userContext?: string | null;
}

export interface DomainWorkflowScriptPreview {
  gate?: {
    issues?: Array<{ severity: string; message?: string; line?: number | null }>;
    [key: string]: unknown;
  };
  permission?: Record<string, unknown>;
  calls?: unknown[];
  [key: string]: unknown;
}

export interface DomainWorkflowDraft {
  template: DomainWorkflowTemplate;
  sessionId: string;
  goalId?: string | null;
  executionMode: string;
  workflowKind: string;
  scriptSource: string;
  scriptPreview: DomainWorkflowScriptPreview;
  requiredEvidence: DomainEvidenceRequirement[];
  approvalGates: DomainApprovalGate[];
  verificationPolicy: DomainVerificationRule[];
  warnings: string[];
}

export type AskUserText =
  | string
  | {
      key: string;
      params?: Record<string, unknown>;
      fallback?: string | null;
    };

export interface AskUserQuestionOptionInput {
  value: string;
  label: AskUserText;
  description?: AskUserText | null;
  recommended?: boolean;
  preview?: string | null;
  previewKind?: string | null;
}

export interface AskUserQuestionInput {
  questionId: string;
  text: AskUserText;
  options: AskUserQuestionOptionInput[];
  allowCustom?: boolean;
  multiSelect?: boolean;
  template?: string | null;
  header?: AskUserText | null;
  timeoutSecs?: number | null;
  defaultValues?: string[];
}

export interface CreateOwnerAskUserQuestionInput {
  sessionId: string;
  questions: AskUserQuestionInput[];
  context?: AskUserText | null;
  source?: string | null;
  timeoutSecs?: number | null;
  ownerResponse: {
    action: "record_domain_evidence";
    domainEvidence: RecordDomainEvidenceInput;
  };
}

export interface RecordDomainEvidenceInput {
  goalId?: string | null;
  sessionId?: string | null;
  projectId?: string | null;
  domain: string;
  evidenceType: string;
  title: string;
  summary?: string | null;
  sourceMetadata?: Record<string, unknown>;
  confidence?: number | null;
  accessScope?: string | null;
  redactionStatus?: string | null;
}

export interface ListDomainEvidenceInput {
  goalId?: string | null;
  sessionId?: string | null;
  projectId?: string | null;
  domain?: string | null;
  evidenceType?: string | null;
  limit?: number | null;
}

export interface DomainEvidenceItem {
  id: string;
  goalId?: string | null;
  sessionId: string;
  projectId?: string | null;
  domain: string;
  evidenceType: string;
  title: string;
  summary?: string | null;
  sourceMetadata: Record<string, unknown>;
  confidence?: number | null;
  accessScope: string;
  redactionStatus: string;
  createdAt: string;
  updatedAt: string;
}

export interface DomainArtifactExportGuardInput {
  goalId?: string | null;
  sessionId?: string | null;
  projectId?: string | null;
  domain?: string | null;
  artifactPath?: string | null;
  artifactTitle?: string | null;
  artifactKind?: string | null;
  requireArtifactCreated?: boolean;
  requireArtifactReviewed?: boolean;
  maxSensitiveUnreviewed?: number | null;
  maxRedactionPending?: number | null;
}

export interface DomainArtifactExportGuardThresholds {
  requireArtifactCreated: boolean;
  requireArtifactReviewed: boolean;
  maxSensitiveUnreviewed: number;
  maxRedactionPending: number;
}

export interface DomainArtifactExportGuardScope {
  scope: string;
  goalId?: string | null;
  sessionId?: string | null;
  projectId?: string | null;
  domain?: string | null;
}

export interface DomainArtifactExportGuardSummary {
  evidenceItems: number;
  artifactCreated: number;
  artifactReviewed: number;
  exportReviewed: number;
  sensitiveEvidence: number;
  sensitiveUnreviewed: number;
  redactionPending: number;
  privateOrConnectorEvidence: number;
}

export interface DomainArtifactExportGuardCheck {
  name: string;
  status: "passed" | "failed" | "insufficient_data" | string;
  severity: string;
  expected: string;
  actual: string;
  detail: string;
}

export interface DomainArtifactExportGuardEvidence {
  id: string;
  evidenceType: string;
  title: string;
  accessScope: string;
  redactionStatus: string;
  createdAt: string;
  reason: string;
}

export interface DomainArtifactExportGuardReport {
  generatedAt: string;
  status: "passed" | "failed" | "insufficient_data" | string;
  scope: DomainArtifactExportGuardScope;
  artifactPath?: string | null;
  artifactTitle?: string | null;
  artifactKind?: string | null;
  thresholds: DomainArtifactExportGuardThresholds;
  summary: DomainArtifactExportGuardSummary;
  checks: DomainArtifactExportGuardCheck[];
  blockers: string[];
  recommendedNextSteps: string[];
  evidenceRequiringReview: DomainArtifactExportGuardEvidence[];
}

export interface DomainConnectorActionGuardInput {
  goalId?: string | null;
  sessionId?: string | null;
  projectId?: string | null;
  domain?: string | null;
  toolName?: string | null;
  connector?: string | null;
  action?: string | null;
  requireExplicitApproval?: boolean;
  requireRollbackPlan?: boolean;
  requireExportGuardForDelivery?: boolean;
}

export interface DomainConnectorActionGuardThresholds {
  requireExplicitApproval: boolean;
  requireRollbackPlan: boolean;
  requireExportGuardForDelivery: boolean;
}

export interface DomainConnectorActionGuardScope {
  scope: string;
  goalId?: string | null;
  sessionId?: string | null;
  projectId?: string | null;
  domain?: string | null;
}

export interface DomainConnectorActionGuardSummary {
  evidenceItems: number;
  actionEvidence: number;
  approvalEvidence: number;
  rollbackEvidence: number;
  sensitiveEvidence: number;
  deliveryAction: boolean;
  exportGuardStatus?: string | null;
}

export interface DomainConnectorActionGuardCheck {
  name: string;
  status: "passed" | "failed" | "insufficient_data" | string;
  severity: string;
  expected: string;
  actual: string;
  detail: string;
}

export interface DomainConnectorActionGuardEvidence {
  id: string;
  evidenceType: string;
  title: string;
  accessScope: string;
  redactionStatus: string;
  createdAt: string;
  reason: string;
}

export interface DomainConnectorActionGuardReport {
  generatedAt: string;
  status: "passed" | "failed" | "insufficient_data" | string;
  scope: DomainConnectorActionGuardScope;
  toolName?: string | null;
  connector?: string | null;
  action?: string | null;
  risk?: string | null;
  thresholds: DomainConnectorActionGuardThresholds;
  summary: DomainConnectorActionGuardSummary;
  checks: DomainConnectorActionGuardCheck[];
  blockers: string[];
  recommendedNextSteps: string[];
  relatedEvidence: DomainConnectorActionGuardEvidence[];
}

export interface DomainConnectorE2EGateInput {
  goalId?: string | null;
  sessionId?: string | null;
  projectId?: string | null;
  domain?: string | null;
  toolName?: string | null;
  connector?: string | null;
  action?: string | null;
  requireConnectorInput?: boolean;
  requireDraft?: boolean;
  requireExplicitApproval?: boolean;
  requireExecutionResult?: boolean;
  requirePostActionVerification?: boolean;
  requireRollbackPlan?: boolean;
  requireExportGuardForDelivery?: boolean;
}

export interface DomainConnectorE2EGateThresholds {
  requireConnectorInput: boolean;
  requireDraft: boolean;
  requireExplicitApproval: boolean;
  requireExecutionResult: boolean;
  requirePostActionVerification: boolean;
  requireRollbackPlan: boolean;
  requireExportGuardForDelivery: boolean;
}

export interface DomainConnectorE2EGateScope {
  scope: string;
  goalId?: string | null;
  sessionId?: string | null;
  projectId?: string | null;
  domain?: string | null;
}

export interface DomainConnectorE2EGateSummary {
  evidenceItems: number;
  connectorInputEvidence: number;
  draftEvidence: number;
  approvalEvidence: number;
  executionEvidence: number;
  verificationEvidence: number;
  rollbackEvidence: number;
  sensitiveEvidence: number;
  deliveryAction: boolean;
  connectorActionGuardStatus?: string | null;
  exportGuardStatus?: string | null;
}

export interface DomainConnectorE2EGateCheck {
  name: string;
  status: "passed" | "failed" | "insufficient_data" | string;
  severity: string;
  expected: string;
  actual: string;
  detail: string;
}

export interface DomainConnectorE2EGateEvidence {
  id: string;
  evidenceType: string;
  title: string;
  accessScope: string;
  redactionStatus: string;
  createdAt: string;
  reason: string;
}

export interface DomainConnectorE2EGateReport {
  generatedAt: string;
  status: "passed" | "failed" | "insufficient_data" | string;
  scope: DomainConnectorE2EGateScope;
  toolName?: string | null;
  connector?: string | null;
  action?: string | null;
  risk?: string | null;
  thresholds: DomainConnectorE2EGateThresholds;
  summary: DomainConnectorE2EGateSummary;
  checks: DomainConnectorE2EGateCheck[];
  blockers: string[];
  recommendedNextSteps: string[];
  relatedEvidence: DomainConnectorE2EGateEvidence[];
}

export interface CodingEvalFixture {
  name: string;
  description?: string;
  task?: CodingTaskEvalSpec | null;
  repo: CodingEvalRepoFixture;
  setup?: Record<string, unknown>;
  runs?: CodingEvalRuns;
  checks?: CodingEvalChecks;
}

export interface CodingEvalRuns {
  execution?: CodingEvalAgentExecutionRun | null;
  task?: Record<string, unknown> | null;
  workflow?: Record<string, unknown> | null;
  review?: Record<string, unknown> | null;
  verification?: Record<string, unknown> | null;
  context?: Record<string, unknown> | null;
  improvement?: Record<string, unknown> | null;
}

export interface CodingEvalAgentExecutionRun {
  mode?: "agent" | "fixture_patch" | string;
  prompt?: string | null;
  agentId?: string | null;
  displayText?: string | null;
  providers?: Record<string, unknown>[];
  modelChain?: CodingEvalActiveModel[];
  compactConfig?: Record<string, unknown> | null;
  reasoningEffort?: string | null;
  extraSystemContext?: string | null;
  deniedTools?: string[];
  autoApproveTools?: boolean;
}

export interface CodingEvalActiveModel {
  providerId: string;
  modelId: string;
}

export interface CodingEvalChecks {
  execution?: CodingEvalAgentExecutionCheck | null;
  task?: Record<string, unknown> | null;
  workflow?: Record<string, unknown> | null;
  review?: Record<string, unknown> | null;
  verification?: Record<string, unknown> | null;
  context?: Record<string, unknown> | null;
  improvement?: Record<string, unknown> | null;
}

export interface CodingEvalAgentExecutionCheck {
  expectedMode?: string | null;
  expectedStatus?: string | null;
  expectedChangedFiles?: string[];
  forbiddenChangedFiles?: string[];
  expectedToolCalls?: string[];
  minToolCalls?: number | null;
  requireTurn?: boolean | null;
  responseContains?: string[];
  errorContains?: string[];
}

export interface CodingTaskEvalSpec {
  id: string;
  taskType?: string;
  title: string;
  source?: string;
  prompt: string;
  executionMode?: string;
  expectedBehavior?: string[];
  forbiddenBehavior?: string[];
  likelyFiles?: string[];
  expectedArtifacts?: string[];
  requiresSeededState?: boolean;
  allowedValidation?: string[];
  successCriteria?: string[];
  failureNotes?: string[];
}

export interface CodingEvalRepoFixture {
  files?: CodingEvalFileFixture[];
  changes?: CodingEvalFileFixture[];
}

export interface CodingEvalFileFixture {
  path: string;
  text: string;
}

export interface CodingEvalFixtureReport {
  name: string;
  metrics: CodingEvalMetrics;
  outcomes: CodingEvalCheckOutcome[];
  execution?: CodingEvalAgentExecutionReport | null;
  task?: CodingTaskEvalReport | null;
}

export interface CodingEvalCheckOutcome {
  name: string;
  passed: boolean;
  detail: string;
}

export interface CodingEvalMetrics {
  contextPrecision?: number | null;
  criticalContextRecall?: number | null;
  reviewFindings?: number | null;
  verificationCommands: string[];
  executionStatus?: string | null;
  executionMode?: string | null;
  executionChangedFiles: string[];
  executionToolCalls: string[];
  taskOutcome?: string | null;
  taskScore?: number | null;
  taskFailureCategory?: string | null;
  taskChangedFiles: string[];
  taskConstraintViolations: number;
}

export interface CodingEvalAgentExecutionReport {
  mode: string;
  status: string;
  prompt: string;
  agentId: string;
  turnId?: string | null;
  response?: string | null;
  error?: string | null;
  modelUsed?: CodingEvalActiveModel | null;
  toolCalls: string[];
  changedFiles: string[];
  diffBytes: number;
}

export interface CodingTaskEvalReport {
  taskId: string;
  taskType: string;
  title: string;
  outcome: string;
  score: number;
  failureCategory?: string | null;
  diff: CodingTaskDiffSummary;
  validation: CodingTaskValidationSummary;
  review: CodingTaskReviewSummary;
  context: CodingTaskContextSummary;
  goal: CodingTaskGoalSummary;
  checks: CodingTaskEvalCheckResult[];
  metrics: Record<string, unknown>;
}

export interface CodingTaskDiffSummary {
  changedFiles: string[];
  filesChanged: number;
  insertions: number;
  deletions: number;
  diffBytes: number;
}

export interface CodingTaskValidationSummary {
  commands: string[];
  commandCount: number;
  allowedCommandCount: number;
  disallowedCommands: string[];
}

export interface CodingTaskReviewSummary {
  requested: boolean;
  findings: number;
  blockingFindings: number;
}

export interface CodingTaskContextSummary {
  requested: boolean;
  candidates: number;
  requiredContextRecall?: number | null;
}

export interface CodingTaskGoalSummary {
  requested: boolean;
  evaluated: boolean;
  state?: string | null;
  evidenceRelations: string[];
}

export interface CodingTaskEvalCheckResult {
  name: string;
  passed: boolean;
  detail: string;
  category: string;
  severity: string;
}

export interface RecordCodingEvalRunInput {
  sessionId?: string | null;
  projectId?: string | null;
  suite: string;
  name: string;
  status: string;
  metrics?: Record<string, unknown>;
  sourceType?: string | null;
  sourceId?: string | null;
}

export interface CodingEvalRunRecord extends RecordCodingEvalRunInput {
  id: string;
  sessionId?: string | null;
  projectId?: string | null;
  metrics: Record<string, unknown>;
  sourceType?: string | null;
  sourceId?: string | null;
  createdAt: string;
}

export interface GitInfo {
  branch: string | null;
  worktrees: WorktreeInfo[];
}

/**
 * Returns `true` when the app is running inside a Tauri webview.
 *
 * Detection is based on the presence of `window.__TAURI_INTERNALS__` which
 * Tauri injects before any user script executes.
 */
export function isTauriMode(): boolean {
  try {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    return typeof window !== "undefined" && !!(window as any).__TAURI_INTERNALS__;
  } catch {
    return false;
  }
}

/**
 * Outcome of a local image picker interaction.
 *
 * - `src`: URL the caller can pass to `<img src>` or into `AvatarCropDialog`.
 *   In Tauri mode this is a `tauri://` asset URL (safe to display but the
 *   final Blob comes from the crop dialog's canvas); in HTTP mode this is
 *   a `blob:` URL created from a `<input type="file">` selection.
 * - `file`: the underlying `File` in HTTP mode. Absent in Tauri mode (the
 *   crop dialog re-encodes whatever the canvas produces before upload).
 * - `revoke`: release the URL when the caller is done previewing. Safe to
 *   call multiple times. Must be called on crop confirm, cancel, and on
 *   component unmount to avoid leaking Blob-backed memory in long HTTP
 *   sessions.
 */
export interface PickedImage {
  src: string;
  file?: File;
  revoke?: () => void;
}

/**
 * Prompt the user to pick a single local image, returning either an
 * HTTP-displayable `src` plus the underlying `File`, or `null` when the
 * user cancels.
 *
 * Transport-specific implementations are provided by:
 *  - [`pickLocalImage`](./transport-tauri.ts) — `@tauri-apps/plugin-dialog.open` + `convertFileSrc`.
 *  - [`pickLocalImage`](./transport-http.ts) — hidden `<input type="file">`.
 *
 * Callers obtain the right one via `getTransport().pickLocalImage()` (this
 * method is on the Transport interface below; there's also a re-export
 * here so the type is co-located with `PickedImage`).
 */
export type PickLocalImageFn = () => Promise<PickedImage | null>;

/**
 * Normalize a `listen()` payload into its decoded form.
 *
 * Tauri 2 and the HTTP/WS transports both deliver already-parsed JS values,
 * but older backend paths that explicitly `serde_json::to_string(...)` before
 * emitting still arrive as a JSON string. This helper handles both shapes so
 * call sites don't need to repeat the `typeof raw === "string"` check.
 *
 * Returns `null` for any payload that isn't a decodable object (`undefined` /
 * `null` / a non-object primitive / an unparseable string). Every call site in
 * this app expects an object shape, so a malformed or empty frame must surface
 * as `null` rather than poison downstream `payload.x` access. This guard is the
 * root-cause fix for crashes like "undefined is not an object (evaluating
 * 't.jobId')" seen when the macOS WebView flushes a stray event after wake.
 */
export function parsePayload<T>(raw: unknown): T | null {
  let value: unknown = raw;
  if (typeof raw === "string") {
    try {
      value = JSON.parse(raw);
    } catch {
      return null;
    }
  }
  return value !== null && typeof value === "object" ? (value as T) : null;
}
