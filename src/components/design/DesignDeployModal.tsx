import { useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { Cloud, Loader2, ExternalLink, Copy, Check, Globe } from "lucide-react"

import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { cn } from "@/lib/utils"
import { SecretInput } from "@/components/ui/secret-input"
import { Button } from "@/components/ui/button"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { toast } from "sonner"

interface Props {
  open: boolean
  onClose: () => void
  artifactId: string | null
}

/**
 * Cloudflare Pages 一键部署对话框（B7-2，opt-in）。首次填 API token + Account ID（token 0600
 * 存 credentials，读时脱敏回填 mask 哨兵，保存传 mask = 保留原 token 不改）。部署产物干净自包含
 * HTML → 返回 pages.dev 公开 URL + 复制。
 */
export function DesignDeployModal({ open, onClose, artifactId }: Props) {
  const { t } = useTranslation()
  const [accountId, setAccountId] = useState("")
  const [token, setToken] = useState("")
  const [mask, setMask] = useState("")
  const [hasToken, setHasToken] = useState(false)
  const [deploying, setDeploying] = useState(false)
  const [url, setUrl] = useState<string | null>(null)
  const [copied, setCopied] = useState(false)
  // 字段级校验：CF Account ID 是 32 位十六进制。填了但格式不对即标红（touched 后才提示，不空态就唠叨）。
  const [accountTouched, setAccountTouched] = useState(false)
  const accountInvalid =
    accountTouched && accountId.trim().length > 0 && !/^[0-9a-fA-F]{32}$/.test(accountId.trim())
  // 自定义域名（决策增量）：绑定后须自行 CNAME 到 *.pages.dev，status 反映验证态（pending→active）。
  const [domainInput, setDomainInput] = useState("")
  const [domains, setDomains] = useState<{ name: string; status: string }[]>([])
  const [bindingDomain, setBindingDomain] = useState(false)
  const domainValid = /^[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$/.test(domainInput.trim())

  // 渲染期重置：打开时清结果（避免 effect 内同步 setState）。
  const [prevOpen, setPrevOpen] = useState(false)
  if (open !== prevOpen) {
    setPrevOpen(open)
    if (open) setUrl(null)
  }

  useEffect(() => {
    if (!open) return
    let cancelled = false
    void getTransport()
      .call<{ accountId: string; hasToken: boolean; tokenMask: string }>("get_cf_deploy_config_cmd")
      .then((c) => {
        if (cancelled) return
        setAccountId(c.accountId || "")
        setHasToken(!!c.hasToken)
        setMask(c.tokenMask || "")
        setToken(c.hasToken ? c.tokenMask || "" : "")
      })
      .catch((e) => logger.error("design", "DesignDeployModal", "load config failed", e))
    // 已绑定的自定义域名 + 验证状态（项目未部署过 → 空）。
    if (artifactId) {
      void getTransport()
        .call<{ name: string; status: string }[]>("list_design_domains_cmd", { artifactId })
        .then((list) => {
          if (!cancelled) setDomains(Array.isArray(list) ? list : [])
        })
        .catch(() => {
          /* 未部署 / 无网 → 忽略 */
        })
    }
    return () => {
      cancelled = true
    }
  }, [open])

  const deploy = async () => {
    if (!artifactId || deploying) return
    setDeploying(true)
    try {
      // 只在填了**真正的新 token**（非空、非 mask）时才覆写；否则一律送 mask 让后端保留原
      // token——否则清空预填的 mask 字段会把已存凭据写成空、抹掉（review 修复）。
      const tokenToSave = token.trim() && token !== mask ? token : mask
      await getTransport().call("save_cf_deploy_config_cmd", { apiToken: tokenToSave, accountId })
      const res = await getTransport().call<{ url: string }>("deploy_design_artifact_cmd", {
        artifactId,
      })
      setUrl(res.url)
      try {
        await navigator.clipboard.writeText(res.url)
      } catch {
        /* 剪贴板不可用 → URL 已展示 */
      }
      toast.success(t("design.deploy.done", "已部署，链接已复制"))
    } catch (e) {
      logger.error("design", "DesignDeployModal", "deploy failed", e)
      const msg = String((e as Error)?.message || e).slice(0, 160)
      toast.error(t("design.deploy.failed", "部署失败：{{msg}}", { msg }))
    } finally {
      setDeploying(false)
    }
  }

  const bindDomain = async () => {
    const domain = domainInput.trim()
    if (!artifactId || !domainValid || bindingDomain) return
    setBindingDomain(true)
    try {
      const d = await getTransport().call<{ name: string; status: string }>("bind_design_domain_cmd", {
        artifactId,
        domain,
      })
      setDomains((prev) => [...prev.filter((x) => x.name !== d.name), d])
      setDomainInput("")
      toast.success(t("design.deploy.domainBound", "已绑定域名，请把它 CNAME 到 *.pages.dev 以完成验证"))
    } catch (e) {
      logger.error("design", "DesignDeployModal", "bind domain failed", e)
      const msg = String((e as Error)?.message || e).slice(0, 160)
      toast.error(t("design.deploy.domainFailed", "绑定失败：{{msg}}", { msg }))
    } finally {
      setBindingDomain(false)
    }
  }

  // 可部署 = 有 account 且（已存 token 或填了真正的新 token）。清空字段但已有 token 仍可部署
  // （送 mask 保留）；无 token 且字段空/仅 mask 则禁用（须先输入 token）。
  const canDeploy =
    !!accountId.trim() && (hasToken || (!!token.trim() && token !== mask)) && !deploying

  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Cloud className="h-4 w-4 text-primary" />
            {t("design.deploy.title", "部署到 Cloudflare Pages")}
          </DialogTitle>
        </DialogHeader>

        <div className="space-y-3">
          <p className="text-xs text-muted-foreground">
            {t(
              "design.deploy.hint",
              "把这个设计发布成公开网页（*.pages.dev）。需要一个 Cloudflare API Token（Pages 编辑权限）和 Account ID，只保存在本机、加密存放。",
            )}
          </p>
          <div className="space-y-1">
            <label htmlFor="design-deploy-account" className="text-xs font-medium">
              {t("design.deploy.accountId", "Account ID")}
            </label>
            <Input
              id="design-deploy-account"
              value={accountId}
              onChange={(e) => setAccountId(e.target.value)}
              onBlur={() => setAccountTouched(true)}
              placeholder="e.g. 0a1b2c3d…"
              aria-invalid={accountInvalid}
              aria-describedby={accountInvalid ? "design-deploy-account-err" : undefined}
              className={cn(
                "h-8 text-xs",
                accountInvalid && "border-destructive focus-visible:ring-destructive",
              )}
            />
            {accountInvalid && (
              <p id="design-deploy-account-err" role="alert" className="text-[11px] text-destructive">
                {t("design.deploy.accountFormat", "Account ID 应为 32 位十六进制字符")}
              </p>
            )}
          </div>
          <div className="space-y-1">
            <label htmlFor="design-deploy-token" className="flex items-center justify-between text-xs font-medium">
              {t("design.deploy.token", "API Token")}
              <a
                href="https://dash.cloudflare.com/profile/api-tokens"
                target="_blank"
                rel="noopener noreferrer"
                className="inline-flex items-center gap-0.5 text-[11px] font-normal text-primary hover:underline"
              >
                {t("design.deploy.getToken", "获取 Token")}
                <ExternalLink className="h-3 w-3" />
              </a>
            </label>
            <SecretInput
              id="design-deploy-token"
              value={token}
              onChange={(v) => setToken(v)}
              placeholder={hasToken ? mask : t("design.deploy.tokenPh", "粘贴 API Token")}
              className="h-8 text-xs"
            />
          </div>

          {url && (
            <div className="flex items-center gap-2 rounded-lg border border-emerald-500/30 bg-emerald-500/5 px-2.5 py-2">
              <a
                href={url}
                target="_blank"
                rel="noopener noreferrer"
                className="min-w-0 flex-1 truncate text-xs text-emerald-600 hover:underline dark:text-emerald-400"
              >
                {url}
              </a>
              <Button
                variant="ghost"
                size="icon"
                className="h-6 w-6 shrink-0"
                onClick={async () => {
                  try {
                    await navigator.clipboard.writeText(url)
                    setCopied(true)
                    window.setTimeout(() => setCopied(false), 1500)
                  } catch {
                    /* noop */
                  }
                }}
              >
                {copied ? (
                  <Check className="h-3.5 w-3.5 text-emerald-500" />
                ) : (
                  <Copy className="h-3.5 w-3.5" />
                )}
              </Button>
            </div>
          )}

          {(url || domains.length > 0) && (
            <div className="space-y-2 rounded-lg border border-border/60 bg-muted/30 p-3">
              <div className="flex items-center gap-1.5 text-xs font-medium text-foreground">
                <Globe className="h-3.5 w-3.5 text-muted-foreground" />
                {t("design.deploy.customDomain", "自定义域名")}
              </div>
              <p className="text-[11px] leading-relaxed text-muted-foreground">
                {t(
                  "design.deploy.domainHint",
                  "绑定你自己的域名，然后把它 CNAME 到 *.pages.dev；验证通过后状态转为 active。",
                )}
              </p>
              {domains.length > 0 && (
                <ul className="space-y-1">
                  {domains.map((d) => (
                    <li
                      key={d.name}
                      className="flex items-center justify-between gap-2 rounded-md bg-background/60 px-2 py-1"
                    >
                      <span className="min-w-0 flex-1 truncate text-xs">{d.name}</span>
                      <span
                        className={cn(
                          "shrink-0 rounded px-1.5 py-0.5 text-[10px] font-medium",
                          d.status === "active"
                            ? "bg-emerald-500/15 text-emerald-600 dark:text-emerald-400"
                            : "bg-amber-500/15 text-amber-600 dark:text-amber-400",
                        )}
                      >
                        {d.status === "active"
                          ? t("design.deploy.domainActive", "已生效")
                          : t("design.deploy.domainPending", "待验证")}
                      </span>
                    </li>
                  ))}
                </ul>
              )}
              <div className="flex items-center gap-2">
                <Input
                  value={domainInput}
                  onChange={(e) => setDomainInput(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" && domainValid && !bindingDomain) void bindDomain()
                  }}
                  placeholder={t("design.deploy.domainPlaceholder", "例如 design.example.com")}
                  className="h-8 flex-1 text-xs"
                  spellCheck={false}
                  autoCapitalize="none"
                  autoCorrect="off"
                />
                <Button
                  size="sm"
                  variant="secondary"
                  className="h-8 shrink-0"
                  disabled={!domainValid || bindingDomain}
                  onClick={() => void bindDomain()}
                >
                  {bindingDomain ? (
                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  ) : (
                    t("design.deploy.bindDomain", "绑定")
                  )}
                </Button>
              </div>
            </div>
          )}
        </div>

        <DialogFooter>
          <Button variant="ghost" onClick={onClose}>
            {t("common.close", "关闭")}
          </Button>
          <Button onClick={() => void deploy()} disabled={!canDeploy}>
            {deploying ? (
              <Loader2 className="mr-1.5 h-4 w-4 animate-spin" />
            ) : (
              <Cloud className="mr-1.5 h-4 w-4" />
            )}
            {t("design.deploy.deploy", "部署")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

export default DesignDeployModal
