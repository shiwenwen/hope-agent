import { useCallback, useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { Cloud, Loader2, ExternalLink, Copy, Check, Globe, AlertTriangle, CheckCircle2 } from "lucide-react"

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
  // 多提供商部署：Cloudflare Pages（默认）/ Vercel，各自独立凭据 + 部署路径。
  const [provider, setProvider] = useState<"cloudflare" | "vercel">("cloudflare")
  const [vercelToken, setVercelToken] = useState("")
  const [vercelMask, setVercelMask] = useState("")
  const [vercelHasToken, setVercelHasToken] = useState(false)
  const [teamId, setTeamId] = useState("")
  // 部署预检（provider 无关，查产物本身）：errors 阻断部署，warnings 仅提示。
  const [preflight, setPreflight] = useState<{
    ok: boolean
    sizeBytes: number
    warnings: string[]
    errors: string[]
  } | null>(null)
  // 部署历史（跨 provider，最新在前）。
  const [deployments, setDeployments] = useState<
    { provider: string; url: string; createdAt: string }[]
  >([])

  const loadDeployments = useCallback(() => {
    if (!artifactId) return
    void getTransport()
      .call<{ provider: string; url: string; createdAt: string }[]>("list_design_deployments_cmd", {
        artifactId,
      })
      .then((list) => setDeployments(Array.isArray(list) ? list : []))
      .catch(() => {
        /* 无历史 / 无网 → 忽略 */
      })
  }, [artifactId])

  // 渲染期重置：打开时清结果（避免 effect 内同步 setState）。
  const [prevOpen, setPrevOpen] = useState(false)
  if (open !== prevOpen) {
    setPrevOpen(open)
    if (open) setUrl(null)
  }

  useEffect(() => {
    if (!open) return
    let cancelled = false
    // 部署预检（provider 无关）：产物空 / 超限 / 外部引用。
    if (artifactId) {
      void getTransport()
        .call<{ ok: boolean; sizeBytes: number; warnings: string[]; errors: string[] }>(
          "preflight_design_deploy_cmd",
          { artifactId },
        )
        .then((r) => {
          if (!cancelled) setPreflight(r)
        })
        .catch(() => {
          if (!cancelled) setPreflight(null)
        })
      loadDeployments()
    }
    if (provider === "cloudflare") {
      void getTransport()
        .call<{ accountId: string; hasToken: boolean; tokenMask: string }>("get_cf_deploy_config_cmd")
        .then((c) => {
          if (cancelled) return
          setAccountId(c.accountId || "")
          setHasToken(!!c.hasToken)
          setMask(c.tokenMask || "")
          setToken(c.hasToken ? c.tokenMask || "" : "")
        })
        .catch((e) => logger.error("design", "DesignDeployModal", "load cf config failed", e))
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
    } else {
      void getTransport()
        .call<{ teamId: string; hasToken: boolean; tokenMask: string }>("get_vercel_deploy_config_cmd")
        .then((c) => {
          if (cancelled) return
          setTeamId(c.teamId || "")
          setVercelHasToken(!!c.hasToken)
          setVercelMask(c.tokenMask || "")
          setVercelToken(c.hasToken ? c.tokenMask || "" : "")
        })
        .catch((e) => logger.error("design", "DesignDeployModal", "load vercel config failed", e))
    }
    return () => {
      cancelled = true
    }
  }, [open, provider, artifactId, loadDeployments])

  const deploy = async () => {
    if (!artifactId || deploying) return
    setDeploying(true)
    try {
      // 只在填了**真正的新 token**（非空、非 mask）时才覆写；否则一律送 mask 让后端保留原
      // token——否则清空预填的 mask 字段会把已存凭据写成空、抹掉（review 修复）。
      let res: { url: string }
      if (provider === "vercel") {
        const tokenToSave = vercelToken.trim() && vercelToken !== vercelMask ? vercelToken : vercelMask
        await getTransport().call("save_vercel_deploy_config_cmd", { apiToken: tokenToSave, teamId })
        res = await getTransport().call<{ url: string }>("deploy_design_artifact_vercel_cmd", {
          artifactId,
        })
      } else {
        const tokenToSave = token.trim() && token !== mask ? token : mask
        await getTransport().call("save_cf_deploy_config_cmd", { apiToken: tokenToSave, accountId })
        res = await getTransport().call<{ url: string }>("deploy_design_artifact_cmd", {
          artifactId,
        })
      }
      setUrl(res.url)
      loadDeployments()
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

  // 可部署 = 凭据齐（已存 token 或填了真正的新 token）。清空字段但已有 token 仍可部署
  // （送 mask 保留）；无 token 且字段空/仅 mask 则禁用（须先输入 token）。CF 另需 account。
  const canDeploy =
    !deploying &&
    (!preflight || preflight.ok) &&
    (provider === "vercel"
      ? vercelHasToken || (!!vercelToken.trim() && vercelToken !== vercelMask)
      : !!accountId.trim() && (hasToken || (!!token.trim() && token !== mask)))

  // 切 provider 清上一次结果，避免展示错平台的 URL。
  const switchProvider = (p: "cloudflare" | "vercel") => {
    if (p === provider) return
    setProvider(p)
    setUrl(null)
  }

  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Cloud className="h-4 w-4 text-primary" />
            {provider === "vercel"
              ? t("design.deploy.titleVercel", "部署到 Vercel")
              : t("design.deploy.title", "部署到 Cloudflare Pages")}
          </DialogTitle>
        </DialogHeader>

        <div className="space-y-3">
          {/* 提供商切换：CF Pages / Vercel */}
          <div className="grid grid-cols-2 gap-1 rounded-lg bg-muted/50 p-1">
            {(["cloudflare", "vercel"] as const).map((p) => (
              <button
                key={p}
                type="button"
                onClick={() => switchProvider(p)}
                className={cn(
                  "rounded-md px-2 py-1.5 text-xs font-medium transition-colors",
                  provider === p
                    ? "bg-background text-foreground shadow-sm"
                    : "text-muted-foreground hover:text-foreground",
                )}
              >
                {p === "vercel" ? "Vercel" : "Cloudflare Pages"}
              </button>
            ))}
          </div>

          <p className="text-xs text-muted-foreground">
            {provider === "vercel"
              ? t(
                  "design.deploy.hintVercel",
                  "把这个设计发布成公开网页（*.vercel.app）。需要一个 Vercel API Token，只保存在本机、加密存放。",
                )
              : t(
                  "design.deploy.hint",
                  "把这个设计发布成公开网页（*.pages.dev）。需要一个 Cloudflare API Token（Pages 编辑权限）和 Account ID，只保存在本机、加密存放。",
                )}
          </p>

          {/* 部署预检就绪状态：阻断（红）/ 告警（琥珀）/ 就绪（绿） */}
          {preflight && !preflight.ok && (
            <div className="flex items-start gap-2 rounded-lg border border-destructive/40 bg-destructive/5 px-2.5 py-2 text-[11px] text-destructive">
              <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
              <div className="min-w-0">
                <p className="font-medium">{t("design.deploy.preflightBlocked", "无法部署")}</p>
                <ul className="mt-0.5 list-inside list-disc space-y-0.5">
                  {preflight.errors.map((e, i) => (
                    <li key={i}>{e}</li>
                  ))}
                </ul>
              </div>
            </div>
          )}
          {preflight?.ok && preflight.warnings.length > 0 && (
            <div className="flex items-start gap-2 rounded-lg border border-amber-500/40 bg-amber-500/5 px-2.5 py-2 text-[11px] text-amber-600 dark:text-amber-400">
              <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
              <ul className="min-w-0 list-inside list-disc space-y-0.5">
                {preflight.warnings.map((w, i) => (
                  <li key={i}>{w}</li>
                ))}
              </ul>
            </div>
          )}
          {preflight?.ok && preflight.warnings.length === 0 && (
            <div className="flex items-center gap-1.5 text-[11px] text-emerald-600 dark:text-emerald-400">
              <CheckCircle2 className="h-3.5 w-3.5 shrink-0" />
              {t("design.deploy.preflightReady", "产物已就绪，可部署")}
            </div>
          )}

          {provider === "cloudflare" ? (
            <>
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
            </>
          ) : (
            <>
              <div className="space-y-1">
                <label
                  htmlFor="design-deploy-vercel-token"
                  className="flex items-center justify-between text-xs font-medium"
                >
                  {t("design.deploy.token", "API Token")}
                  <a
                    href="https://vercel.com/account/tokens"
                    target="_blank"
                    rel="noopener noreferrer"
                    className="inline-flex items-center gap-0.5 text-[11px] font-normal text-primary hover:underline"
                  >
                    {t("design.deploy.getToken", "获取 Token")}
                    <ExternalLink className="h-3 w-3" />
                  </a>
                </label>
                <SecretInput
                  id="design-deploy-vercel-token"
                  value={vercelToken}
                  onChange={(v) => setVercelToken(v)}
                  placeholder={vercelHasToken ? vercelMask : t("design.deploy.tokenPh", "粘贴 API Token")}
                  className="h-8 text-xs"
                />
              </div>
              <div className="space-y-1">
                <label htmlFor="design-deploy-team" className="text-xs font-medium">
                  {t("design.deploy.teamId", "Team ID（团队账号可选）")}
                </label>
                <Input
                  id="design-deploy-team"
                  value={teamId}
                  onChange={(e) => setTeamId(e.target.value)}
                  placeholder={t("design.deploy.teamIdPh", "个人账号留空")}
                  className="h-8 text-xs"
                  spellCheck={false}
                  autoCapitalize="none"
                  autoCorrect="off"
                />
              </div>
            </>
          )}

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

          {provider === "cloudflare" && (url || domains.length > 0) && (
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

          {deployments.length > 0 && (
            <div className="space-y-1.5">
              <div className="text-xs font-medium text-muted-foreground">
                {t("design.deploy.history", "部署历史")}
              </div>
              <ul className="max-h-32 space-y-1 overflow-y-auto">
                {deployments.map((d, i) => (
                  <li
                    key={`${d.url}-${i}`}
                    className="flex items-center gap-2 rounded-md bg-muted/40 px-2 py-1 text-[11px]"
                  >
                    <span className="shrink-0 rounded bg-background/70 px-1 py-0.5 font-medium text-muted-foreground">
                      {d.provider === "vercel" ? "Vercel" : "CF"}
                    </span>
                    <a
                      href={d.url}
                      target="_blank"
                      rel="noopener noreferrer"
                      className="min-w-0 flex-1 truncate text-primary hover:underline"
                    >
                      {d.url}
                    </a>
                    <span className="shrink-0 text-muted-foreground">
                      {new Date(d.createdAt).toLocaleString()}
                    </span>
                  </li>
                ))}
              </ul>
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
