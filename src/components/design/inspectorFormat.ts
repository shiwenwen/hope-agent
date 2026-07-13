/**
 * 尺寸/长度类字段的**显示层**格式化（只影响输入框里看到的值，不改回写逻辑）：
 * ① 把 computed 出来的「未设」关键字（`none` / `auto` / `normal`）显示为空 —— 露出友好占位
 *    （如「不限」/「自动」），避免用户把这些 CSS 关键字当成自己设过的值；
 * ② 把 sub-pixel 长度（如 `563.90625px`）四舍五入到 2 位，避免检查器塞满超长小数。
 * 在**调用点**格式化（喂给 TextRow 的 value），使脏值守卫的基线与显示一致：不动它就不 commit。
 */
export function formatSizeDisplay(raw: string): string {
  const v = raw.trim()
  if (!v || v === "none" || v === "auto" || v === "normal") return ""
  const m = /^(-?\d*\.?\d+)(px|%|em|rem|vh|vw)$/.exec(v)
  if (m) {
    const n = Number(m[1])
    if (Number.isFinite(n)) return `${Math.round(n * 100) / 100}${m[2]}`
  }
  return v
}
