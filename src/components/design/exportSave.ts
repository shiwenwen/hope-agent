/**
 * 设计空间导出「保存结果 → 提示」统一编排（DesignView / DesignTokenExport 共用）。
 *
 * 把 `Transport.saveFileAs` 的 {@link SaveResult} 收敛成一致的 toast 行为：
 * - 桌面保存成功（有 path）→ 成功 toast + 「在文件夹中显示」动作（reveal）。
 * - 网页 File System Access 保存成功（无 path）→ 纯成功 toast（沙箱无法 reveal）。
 * - 回退浏览器下载 → 「已下载到浏览器下载目录」toast。
 * - 用户取消保存框 → 只关掉 loading toast，不弹成功/错误。
 */
import { toast } from "sonner"
import type { TFunction } from "i18next"
import type { SaveResult, Transport } from "@/lib/transport"
import { logger } from "@/lib/logger"

export interface PresentSaveOpts {
  /** 复用已存在的 loading toast id（导出耗时的强路格式会先弹 loading）。 */
  toastId?: string | number
  /** 成功文案覆盖（默认「已导出」；分享导出用「已导出可分享的 HTML」）。 */
  savedMsg?: string
}

export function presentSaveResult(
  res: SaveResult,
  tx: Transport,
  t: TFunction,
  opts?: PresentSaveOpts,
): void {
  const id = opts?.toastId
  if (res.status === "canceled") {
    if (id !== undefined) toast.dismiss(id)
    return
  }
  const savedMsg = opts?.savedMsg ?? t("design.ok.exported", "已导出")
  if (res.status === "saved" && res.path) {
    const path = res.path
    toast.success(savedMsg, {
      id,
      action: {
        label: t("design.export.reveal", "在文件夹中显示"),
        onClick: () => {
          void tx.revealFile(path).catch((e) => {
            logger.error("design", "presentSaveResult", "reveal failed", e)
            toast.error(t("design.err.reveal", "打开失败"))
          })
        },
      },
    })
  } else if (res.status === "saved") {
    toast.success(savedMsg, { id })
  } else {
    // downloaded → 浏览器下载目录（网页端无法给本机路径）。
    toast.success(t("design.export.downloaded", "已下载到浏览器下载目录"), { id })
  }
}
