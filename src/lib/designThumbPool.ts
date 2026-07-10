/**
 * 设计缩略图 iframe keep-alive 池（Wave 2-⑦）。
 *
 * 目标：大库滚动时不无限累积挂载的缩略图 iframe。**红线（对抗 review 修复）**：**可见缩略图
 * 永不逐**——只对**滚出视口**的 keep-alive 项设预算 `MAX_KEEPALIVE`、超出按 LRU 逐（被逐者
 * 回退占位；因其已不在视口，无可见回退；滚回时 IntersectionObserver 再触发重挂）。滚动掉帧
 * 的真正治法是 ArtifactThumb 的 arm-linger（进视口 350ms 才挂，快速滚过不挂）；本池只管
 * 「离屏已加载的 iframe 保留多少」的内存上限，绝不把视口内的缩略图打回占位。
 */
const MAX_KEEPALIVE = 16

interface Entry {
  release: () => void
  touched: number
  visible: boolean
}

const live = new Map<string, Entry>()
let clock = 0

// 只逐**不可见**的 keep-alive 项，超预算按 LRU 逐；可见项一律保留。
function evictOffscreen(): void {
  const offscreen: Array<[string, Entry]> = []
  for (const entry of live) {
    if (!entry[1].visible) offscreen.push(entry)
  }
  if (offscreen.length <= MAX_KEEPALIVE) return
  offscreen.sort((a, b) => a[1].touched - b[1].touched) // 最久未触达在前
  for (let i = 0; i < offscreen.length - MAX_KEEPALIVE; i++) {
    const [k, e] = offscreen[i]
    live.delete(k)
    e.release()
  }
}

/** 请求活体槽；已在池内则刷新触达 + 可见性 + release 回调。可见项永不因此被逐。 */
export function acquireThumb(id: string, release: () => void, visible: boolean): void {
  const e = live.get(id)
  if (e) {
    e.touched = ++clock
    e.visible = visible
    e.release = release
    return
  }
  live.set(id, { release, touched: ++clock, visible })
  evictOffscreen()
}

/** 更新可见性：滚入 → 刷新触达（保活）；滚出 → 转入 keep-alive 并在超预算时触发逐出。 */
export function setThumbVisible(id: string, visible: boolean): void {
  const e = live.get(id)
  if (!e) return
  e.visible = visible
  if (visible) e.touched = ++clock
  else evictOffscreen()
}

/** 组件卸载时退出池。 */
export function releaseThumb(id: string): void {
  live.delete(id)
}
