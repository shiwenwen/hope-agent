/**
 * 右侧面板内容区的上下边缘柔化淡出 —— 让内容滚到面板边界时渐隐、不硬切。
 *
 * 套在「标题栏下方那块铺满的内容区」上即可（mask 作用于该容器的绘制输出，
 * 内部滚动子元素经过顶/底淡出带时自然渐隐，故不要求 mask 直接挂在滚动元素上）。
 *
 * Tauri 用 WebKit,补 `-webkit-mask-image` 兜底;调淡出距离(22px)改这一处即可
 * 让所有面板同步生效。纯 Tailwind 任意值类,无自定义 CSS。
 */
export const PANEL_SCROLL_FADE =
  "[mask-image:linear-gradient(to_bottom,transparent_0,#000_22px,#000_calc(100%_-_22px),transparent_100%)] [-webkit-mask-image:linear-gradient(to_bottom,transparent_0,#000_22px,#000_calc(100%_-_22px),transparent_100%)]"
