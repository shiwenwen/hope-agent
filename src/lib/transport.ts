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
  | "url_source";

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
  warnings: string[];
}

export interface ContextRetrievalSnapshot {
  sessionId: string;
  query?: string | null;
  workspaceRoot?: string | null;
  candidates: ContextCandidate[];
  stats: ContextRetrievalStats;
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

export interface CodingEvalFixture {
  name: string;
  description?: string;
  task?: CodingTaskEvalSpec | null;
  repo: CodingEvalRepoFixture;
  setup?: Record<string, unknown>;
  runs?: Record<string, unknown>;
  checks?: Record<string, unknown>;
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
  taskOutcome?: string | null;
  taskScore?: number | null;
  taskFailureCategory?: string | null;
  taskChangedFiles: string[];
  taskConstraintViolations: number;
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
