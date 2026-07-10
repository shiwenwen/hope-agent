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
      } else {
        char--
        setText(s.slice(0, char))
        if (char <= 0) {
          deleting = false
          scene = (scene + 1) % list.length
        }
      }
      timer = window.setTimeout(tick, deleting ? 28 : 52)
    }
    timer = window.setTimeout(tick, 500)
    return () => window.clearTimeout(timer)
  }, [active, scenes.length])

  return text
}
