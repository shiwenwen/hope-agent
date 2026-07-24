import { isTauriMode } from "@/lib/transport"

/**
 * Linux 桌面壳（Tauri + WebKitGTK）判定。
 *
 * WebKitGTK 的 GPU 合成器在透明窗口 / backdrop-filter / 分数缩放等组合下，
 * 会把滚动内容以错误 scale 光栅化——表现为滚动时文字周期性发虚、停下才
 * 恢复清晰（issue #547）。需要按平台降级这类纯装饰性的合成层触发源。
 *
 * 平台在运行期不变，调用方可放心在模块顶层求值一次并缓存。
 * isTauriMode() 为 true 时必有 window/navigator，无需再判 navigator 存在。
 */
export function isLinuxDesktop(): boolean {
  return isTauriMode() && /\bLinux\b/.test(navigator.userAgent)
}

/**
 * 启动时（首帧渲染前）在 <html> 上打平台标记，供 index.css 的全局降级规则
 * 消费（`html[data-linux-webkit]`）——一条规则覆盖全部 backdrop-blur 表面，
 * 代替逐组件条件类。所有窗口入口（主窗 / quickchat / detached）共用 main.tsx，
 * 只需在那里调用一次。
 */
export function applyPlatformDocumentMarkers(): void {
  if (isLinuxDesktop()) {
    document.documentElement.dataset.linuxWebkit = "true"
  }
}
