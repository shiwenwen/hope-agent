import { useEffect, useRef, useState } from "react"

/**
 * 打字机轮播占位（Wave 2-⑩）：让空首屏 composer「活起来」——逐字打出一组示例 prompt、停顿、
 * 退格、切下一句，循环。驱动 Textarea 原生 `placeholder`（无需叠层对齐、天然 pointer-events-none）。
 *
 * - `active=false`（已有输入 / 已聚焦）→ 停播、返回空串（露出用户输入或无占位）。
 * - `prefers-reduced-motion` → 降级为每 3s 整句切换、不逐字（无障碍）。
 */
export function useTypewriterPlaceholder(scenes: string[], active: boolean): string {
  const [text, setText] = useState("")
  const scenesRef = useRef(scenes)
  scenesRef.current = scenes

  useEffect(() => {
    if (!active || scenes.length === 0) {
      setText("")
      return
    }
    const reduced =
      typeof window !== "undefined" &&
      window.matchMedia?.("(prefers-reduced-motion: reduce)").matches
    let scene = 0
    let char = 0
    let deleting = false
    let timer: number
    const tick = () => {
      const list = scenesRef.current
      const s = list[scene % list.length] ?? ""
      if (reduced) {
        setText(s)
        scene = (scene + 1) % list.length
        timer = window.setTimeout(tick, 3000)
        return
      }
      if (!deleting) {
        char++
        setText(s.slice(0, char))
        if (char >= s.length) {
          deleting = true
          timer = window.setTimeout(tick, 1700) // 打完停顿
          return
        }
      } else if (char <= 1) {
        // 删到最后一字 → **直接切下一句第一个字**，绝不渲染空串。否则外层 `typed || fallback`
        // 会在空帧一次性闪出静态回退占位（一整句、且与打字机不同），每轮切换都闪一下（用户报告）。
        deleting = false
        scene = (scene + 1) % list.length
        char = 1
        setText((list[scene % list.length] ?? "").slice(0, 1))
      } else {
        char--
        setText(s.slice(0, char))
      }
      timer = window.setTimeout(tick, deleting ? 28 : 52)
    }
    timer = window.setTimeout(tick, 500)
    return () => window.clearTimeout(timer)
  }, [active, scenes.length])

  return text
}
