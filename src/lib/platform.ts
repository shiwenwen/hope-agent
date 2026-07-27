import { isTauriMode } from "@/lib/transport"

/**
 * Linux 桌面壳（Tauri + WebKitGTK）判定。
 *
 * 平台在运行期不变，调用方可放心在模块顶层求值一次并缓存。
 * isTauriMode() 为 true 时必有 window/navigator，无需再判 navigator 存在。
 */
export function isLinuxDesktop(): boolean {
  return isTauriMode() && /\bLinux\b/.test(navigator.userAgent)
}
