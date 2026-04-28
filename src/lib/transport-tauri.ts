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
} from "@/lib/transport";
import type { MediaItem } from "@/types/chat";

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

  async revealMedia(item: MediaItem): Promise<void> {
    const path = this.localSourceFor(item);
    if (!path) return;
    await invoke("reveal_in_folder", { path });
  }

  supportsLocalFileOps(): boolean {
    return true;
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
    return { src: convertFileSrc(selected) };
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

  async searchFiles(root: string, q: string, limit?: number): Promise<FileSearchResponse> {
    return invoke<FileSearchResponse>("fs_search_files", {
      root,
      q,
      limit: limit ?? null,
    });
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
