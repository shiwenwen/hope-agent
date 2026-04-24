/**
 * Tauri IPC transport implementation.
 *
 * Wraps `@tauri-apps/api/core` invoke / Channel and
 * `@tauri-apps/api/event` listen into the Transport interface.
 */

import { invoke, Channel, convertFileSrc } from "@tauri-apps/api/core";
import { listen as tauriListen } from "@tauri-apps/api/event";
import type { Transport, ChatStream, PickedImage, DirListing } from "@/lib/transport";
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

  // ----- openChatStream -----

  openChatStream(
    _sessionId: string | null,
    onEvent: (event: string) => void,
  ): ChatStream {
    const channel = new Channel<string>();
    channel.onmessage = (raw: string) => {
      onEvent(raw);
    };

    // The Tauri Channel is passed as an argument to the `chat` command.
    // The actual `invoke("chat", { ..., onEvent: channel })` call is
    // performed by the caller (e.g. useChatStream). Here we just expose
    // the channel so callers can attach it.
    //
    // We store the channel reference on the returned handle so the caller
    // can pass `handle.tauriChannel` to invoke().
    const handle: TauriChatStream = {
      tauriChannel: channel,
      close() {
        // Tauri Channels are closed when the invoke promise resolves or
        // the caller drops the reference. Explicit close is a no-op but
        // we null out the callback to prevent late deliveries.
        channel.onmessage = () => {};
      },
    };

    return handle;
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

  async listServerDirectory(): Promise<DirListing> {
    // Desktop uses the native picker — no server-side listing in this mode.
    throw new Error("listServerDirectory is not available in Tauri mode");
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

    tauriListen(eventName, (event) => {
      handler(event.payload);
    }).then((fn) => {
      if (cancelled) {
        // The caller already unsubscribed before the async setup finished.
        fn();
      } else {
        unlisten = fn;
      }
    });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }
}

/**
 * Extended ChatStream that exposes the underlying Tauri Channel.
 *
 * Callers that need to pass the channel to `invoke("chat", { onEvent })`
 * can narrow the type:
 *
 * ```ts
 * const stream = transport.openChatStream(sid, handler);
 * invoke("chat", { ..., onEvent: (stream as TauriChatStream).tauriChannel });
 * ```
 */
export interface TauriChatStream extends ChatStream {
  readonly tauriChannel: Channel<string>;
}
