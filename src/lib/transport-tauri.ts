/**
 * Tauri IPC transport implementation.
 *
 * Wraps `@tauri-apps/api/core` invoke / Channel and
 * `@tauri-apps/api/event` listen into the Transport interface.
 */

import { invoke, Channel, convertFileSrc } from "@tauri-apps/api/core";
import { listen as tauriListen } from "@tauri-apps/api/event";
import type {
  Transport,
  ChatStartArgs,
  PickedImage,
  DirListing,
  FileSearchResponse,
  ExportSessionArgs,
  ExportSessionResult,
  ExtractedContent,
  FileTextContent,
  ProjectFsScope,
  UploadResult,
  SaveResult,
  SessionArtifacts,
  WorkspaceEnvironmentSnapshot,
} from "@/lib/transport";
import type { FileChangesMetadata, MediaItem } from "@/types/chat";

/** localStorage key remembering the last directory the user saved an export to. */
const EXPORT_DIR_KEY = "design_export_dir";

function readLastExportDir(): string {
  try {
    return localStorage.getItem(EXPORT_DIR_KEY) ?? "";
  } catch {
    return "";
  }
}

function rememberExportDir(savedPath: string): void {
  // Strip the trailing filename segment (handles both / and \ separators).
  const dir = savedPath.replace(/[/\\][^/\\]*$/, "");
  if (!dir || dir === savedPath) return;
  try {
    localStorage.setItem(EXPORT_DIR_KEY, dir);
  } catch {
    /* private mode / quota — best-effort memory only */
  }
}

/** Blob → base64 (no data-uri prefix) via FileReader — native + handles large blobs. */
function blobToBase64(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const s = String(reader.result);
      resolve(s.slice(s.indexOf(",") + 1));
    };
    reader.onerror = () => reject(reader.error ?? new Error("blob read failed"));
    reader.readAsDataURL(blob);
  });
}

export class TauriTransport implements Transport {
  // ----- call -----

  async call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
    return invoke<T>(command, args);
  }

  // ----- prepareFileData -----

  prepareFileData(buffer: ArrayBuffer): number[] {
    return Array.from(new Uint8Array(buffer));
  }

  // ----- startChat -----

  async startChat(
    args: ChatStartArgs,
    onEvent: (event: string) => void,
  ): Promise<string> {
    const channel = new Channel<string>();
    channel.onmessage = onEvent;
    try {
      return await invoke<string>("chat", { ...args, onEvent: channel });
    } finally {
      // Drop the callback so any late-delivered Channel message after the
      // invoke promise resolves does not re-enter caller state.
      channel.onmessage = () => {};
    }
  }

  // ----- media -----

  resolveMediaUrl(item: MediaItem): string | null {
    const source = this.localSourceFor(item);
    return source ? convertFileSrc(source) : null;
  }

  resolveAssetUrl(path: string | null | undefined): string | null {
    if (!path) return null;
    if (
      path.startsWith("data:") ||
      path.startsWith("http://") ||
      path.startsWith("https://")
    ) {
      return path;
    }
    // Absolute path on Unix or Windows — hand to Tauri's asset protocol.
    if (path.startsWith("/") || /^[A-Za-z]:[\\/]/.test(path)) {
      return convertFileSrc(path);
    }
    return null;
  }

  async openMedia(item: MediaItem): Promise<void> {
    const path = this.localSourceFor(item);
    if (!path) return;
    await invoke("open_directory", { path });
  }

  async downloadMedia(item: MediaItem): Promise<void> {
    await this.openMedia(item);
  }

  async openFilePath(path: string): Promise<void> {
    if (!path) return;
    await invoke("open_directory", { path });
  }

  async downloadFilePath(path: string): Promise<void> {
    await this.openFilePath(path);
  }

  async revealMedia(item: MediaItem): Promise<void> {
    const path = this.localSourceFor(item);
    if (!path) return;
    await invoke("reveal_in_folder", { path });
  }

  supportsLocalFileOps(): boolean {
    return true;
  }

  async saveFileAs(blob: Blob, filename: string): Promise<SaveResult> {
    // Native "Save As" dialog, pre-filled with the last-used directory so repeat
    // exports don't force the user to re-navigate each time.
    const { save } = await import("@tauri-apps/plugin-dialog");
    const lastDir = readLastExportDir();
    const sep = lastDir.includes("\\") ? "\\" : "/";
    const defaultPath = lastDir ? `${lastDir.replace(/[/\\]+$/, "")}${sep}${filename}` : filename;
    const path = await save({ defaultPath, title: filename });
    if (!path) return { status: "canceled" };
    // Write via a dedicated Tauri command (user already chose the path through the
    // native dialog). base64 keeps the IPC payload compact for large exports (MP4).
    const dataBase64 = await blobToBase64(blob);
    await invoke("save_exported_file", { path, dataBase64 });
    rememberExportDir(path);
    return { status: "saved", path };
  }

  async revealFile(path: string): Promise<void> {
    if (!path) return;
    await invoke("reveal_in_folder", { path });
  }

  async pickLocalImage(): Promise<PickedImage | null> {
    // Dynamic import so the Tauri-only plugin doesn't show up in the
    // browser bundle when tree-shaking runs against HttpTransport.
    const { open } = await import("@tauri-apps/plugin-dialog");
    const selected = await open({
      multiple: false,
      filters: [
        { name: "Image", extensions: ["png", "jpg", "jpeg", "gif", "webp", "svg"] },
      ],
    });
    if (!selected || typeof selected !== "string") return null;
    return { src: convertFileSrc(selected), path: selected };
  }

  async pickLocalDirectory(): Promise<string | null> {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const selected = await open({ directory: true, multiple: false });
    if (!selected || typeof selected !== "string") return null;
    return selected;
  }

  async listServerDirectory(path?: string): Promise<DirListing> {
    // The chat-input `@` mention popper needs cross-mode parity with HTTP.
    // The working-dir picker still prefers the native dialog, but it can use
    // this too when needed.
    return invoke<DirListing>("fs_list_dir", { path: path ?? null });
  }

  async createDirectory(path: string): Promise<DirListing> {
    return invoke<DirListing>("fs_create_dir", { path });
  }

  async projectFsRawUrl(
    args: ProjectFsScope & { path: string; download?: boolean },
  ): Promise<string | null> {
    // Resolve the workspace-relative path to a canonical absolute path, then
    // hand it to the asset protocol for `<img>` / `<iframe>` preview.
    try {
      const abs = await invoke<string>("project_fs_resolve", {
        scope: args.scope,
        scopeId: args.scopeId,
        path: args.path,
      });
      return abs ? convertFileSrc(abs) : null;
    } catch {
      return null;
    }
  }

  async previewReadText(path: string): Promise<FileTextContent> {
    return invoke<FileTextContent>("preview_read_text", { path });
  }

  async previewExtractDoc(path: string): Promise<ExtractedContent> {
    return invoke<ExtractedContent>("preview_extract", { path });
  }

  async previewRawUrl(path: string): Promise<string | null> {
    // The local asset protocol serves the absolute path directly for
    // `<img>/<iframe>/<video>` preview; `download` is irrelevant on desktop
    // (open/download route through `open_directory`).
    return this.resolveAssetUrl(path);
  }

  async loadSessionArtifacts(sessionId: string): Promise<SessionArtifacts> {
    return invoke<SessionArtifacts>("load_session_artifacts_cmd", { sessionId });
  }

  async loadSessionEnvironment(sessionId: string): Promise<WorkspaceEnvironmentSnapshot> {
    return invoke<WorkspaceEnvironmentSnapshot>("load_session_environment_cmd", { sessionId });
  }

  async loadSessionGitDiff(sessionId: string): Promise<FileChangesMetadata> {
    return invoke<FileChangesMetadata>("load_session_git_diff_cmd", { sessionId });
  }

  async projectFsUpload(
    args: ProjectFsScope & {
      dirPath: string;
      data: Blob;
      fileName: string;
      mimeType?: string;
      overwrite?: boolean;
    },
  ): Promise<UploadResult> {
    // Pass a Uint8Array so Tauri v2 streams the bytes over its binary IPC
    // channel (Rust receives `Vec<u8>`), instead of JSON-encoding a
    // number-per-byte array — which would balloon a 20MB upload into hundreds
    // of MB and freeze the webview.
    const bytes = new Uint8Array(await args.data.arrayBuffer());
    return invoke<UploadResult>("project_fs_upload", {
      scope: args.scope,
      scopeId: args.scopeId,
      dirPath: args.dirPath,
      fileName: args.fileName,
      data: bytes,
      overwrite: args.overwrite ?? false,
    });
  }

  async searchFiles(root: string, q: string, limit?: number): Promise<FileSearchResponse> {
    return invoke<FileSearchResponse>("fs_search_files", {
      root,
      q,
      limit: limit ?? null,
    });
  }

  async exportSession(args: ExportSessionArgs): Promise<ExportSessionResult | null> {
    const { save } = await import("@tauri-apps/plugin-dialog");
    const ext = args.format;
    const defaultName =
      args.defaultFilename && args.defaultFilename.trim().length > 0
        ? args.defaultFilename
        : `session.${ext}`;
    const filterName =
      ext === "md" ? "Markdown" : ext === "json" ? "JSON" : "HTML";
    const savedPath = await save({
      defaultPath: defaultName,
      filters: [{ name: filterName, extensions: [ext] }],
    });
    if (!savedPath) return null;
    await invoke<string>("export_session_cmd", {
      sessionId: args.sessionId,
      format: args.format,
      includeThinking: args.includeThinking,
      includeTools: args.includeTools,
      outputPath: savedPath,
    });
    const filename = savedPath.split(/[\\/]/).pop() ?? defaultName;
    return { filename, savedPath };
  }

  /** Absolute server-side path for Tauri file ops. Legacy items may carry
   *  an absolute path in `url`; items produced after URL migration carry
   *  `/api/attachments/...` there and the absolute path in `localPath`. */
  private localSourceFor(item: MediaItem): string | null {
    if (item.localPath) return item.localPath;
    if (item.url && !item.url.startsWith("/api/")) return item.url;
    return null;
  }

  // ----- listen -----

  listen(eventName: string, handler: (payload: unknown) => void): () => void {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    let cleanedUp = false;

    const cleanup = (fn: () => void) => {
      try {
        void Promise.resolve(fn()).catch((err) => {
          console.warn(`[transport] TauriTransport::listen: failed to unlisten ${eventName}`, err);
        });
      } catch (err) {
        console.warn(`[transport] TauriTransport::listen: failed to unlisten ${eventName}`, err);
      }
    };

    tauriListen(eventName, (event) => {
      handler(event.payload);
    }).then((fn) => {
      if (cancelled) {
        // The caller already unsubscribed before the async setup finished.
        if (!cleanedUp) {
          cleanedUp = true;
          cleanup(fn);
        }
      } else {
        unlisten = fn;
      }
    }).catch((err) => {
      console.warn(`[transport] TauriTransport::listen: failed to listen ${eventName}`, err);
    });

    return () => {
      if (cleanedUp) return;
      cancelled = true;
      if (!unlisten) return;
      cleanedUp = true;
      cleanup(unlisten);
      unlisten = undefined;
    };
  }
}
