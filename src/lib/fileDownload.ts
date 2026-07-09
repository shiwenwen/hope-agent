/**
 * 依赖零耦合的浏览器下载原语（leaf module，不 import transport）。
 *
 * 单独抽出是为了让 transport-http 能直接引用它做「保存失败/无 FS Access 时回退浏览器下载」，
 * 而不必 import `designExport`（后者 import `transport-provider` → 会与 transport-http 形成
 * 循环依赖）。`designExport` 仍 re-export `downloadBlob` 保持旧调用点不变。
 */

/** 触发浏览器下载一个 Blob（Blob URL + 隐藏 `<a download>`）。 */
export function downloadBlob(blob: Blob, filename: string): void {
  const url = URL.createObjectURL(blob)
  const a = document.createElement("a")
  a.href = url
  a.download = filename
  document.body.appendChild(a)
  a.click()
  a.remove()
  setTimeout(() => URL.revokeObjectURL(url), 1000)
}
