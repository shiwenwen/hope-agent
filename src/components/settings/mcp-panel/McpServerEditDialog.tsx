/**
 * Add / Edit MCP server dialog.
 *
 * Form layout (single column so the modal stays narrow):
 * * Name (immutable on edit — renaming would invalidate references)
 * * Enabled switch
 * * Transport kind picker + kind-specific fields
 * * env KV editor (stdio only)
 * * headers KV editor (http/sse/ws only)
 * * Description
 * * Timeouts + concurrency cap
 * * Trust level + auto-approve interlock
 *
 * Save flows through `mcp_add_server` / `mcp_update_server`; on success
 * the parent refreshes the list via `onSaved`.
 */

import { useEffect, useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import { Loader2, Plus, X } from "lucide-react"
import { toast } from "sonner"

import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Switch } from "@/components/ui/switch"
import { Textarea } from "@/components/ui/textarea"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"

import {
  addServer,
  updateServer,
  type McpServerDraft,
  type McpServerSummary,
  type McpTransportKind,
  type McpTrustLevel,
} from "@/lib/mcp"

// ── Form state ───────────────────────────────────────────────────

type FormState = {
  name: string
  enabled: boolean
  kind: McpTransportKind
  command: string
  args: string
  cwd: string
  url: string
  env: Array<[string, string]>
  headers: Array<[string, string]>
  description: string
  connectTimeoutSecs: string
  callTimeoutSecs: string
  maxConcurrentCalls: string
  trustLevel: McpTrustLevel
  autoApprove: boolean
  eager: boolean
  deniedTools: string
  allowedTools: string
}

function initialFromSummary(s: McpServerSummary | null): FormState {
  if (!s) {
    return {
      name: "",
      enabled: true,
      kind: "stdio",
      command: "",
      args: "",
      cwd: "",
      url: "",
      env: [],
      headers: [],
      description: "",
      connectTimeoutSecs: "30",
      callTimeoutSecs: "120",
      maxConcurrentCalls: "4",
      trustLevel: "untrusted",
      autoApprove: false,
      eager: false,
      deniedTools: "",
      allowedTools: "",
    }
  }
  const t = s.transport
  const kind = t.kind
  return {
    name: s.name,
    enabled: s.enabled,
    kind,
    command: kind === "stdio" ? t.command : "",
    args: kind === "stdio" && t.args ? t.args.join("\n") : "",
    cwd: kind === "stdio" ? (t.cwd ?? "") : "",
    url: kind !== "stdio" ? t.url : "",
    env: Object.entries(s.env ?? {}),
    headers: Object.entries(s.headers ?? {}),
    description: s.description ?? "",
    connectTimeoutSecs: String(s.connectTimeoutSecs ?? 30),
    callTimeoutSecs: String(s.callTimeoutSecs ?? 120),
    maxConcurrentCalls: String(s.maxConcurrentCalls ?? 4),
    trustLevel: s.trustLevel ?? "untrusted",
    autoApprove: s.autoApprove,
    eager: s.eager,
    deniedTools: (s.deniedTools ?? []).join("\n"),
    allowedTools: (s.allowedTools ?? []).join("\n"),
  }
}

function formToDraft(form: FormState): McpServerDraft {
  const transport =
    form.kind === "stdio"
      ? {
          kind: "stdio" as const,
          command: form.command.trim(),
          args: form.args
            .split(/\r?\n/)
            .map((s) => s.trim())
            .filter(Boolean),
          cwd: form.cwd.trim() || null,
        }
      : form.kind === "streamableHttp"
        ? { kind: "streamableHttp" as const, url: form.url.trim() }
        : form.kind === "sse"
          ? { kind: "sse" as const, url: form.url.trim() }
          : { kind: "websocket" as const, url: form.url.trim() }

  const env = Object.fromEntries(
    form.env.filter(([k]) => k.trim().length > 0).map(([k, v]) => [k.trim(), v]),
  )
  const headers = Object.fromEntries(
    form.headers.filter(([k]) => k.trim().length > 0).map(([k, v]) => [k.trim(), v]),
  )

  const splitList = (s: string) =>
    s
      .split(/\r?\n/)
      .map((x) => x.trim())
      .filter(Boolean)

  return {
    name: form.name.trim(),
    enabled: form.enabled,
    transport,
    env,
    headers,
    description: form.description.trim() || null,
    connectTimeoutSecs: Number(form.connectTimeoutSecs) || 30,
    callTimeoutSecs: Number(form.callTimeoutSecs) || 120,
    maxConcurrentCalls: Number(form.maxConcurrentCalls) || 4,
    trustLevel: form.trustLevel,
    autoApprove: form.autoApprove,
    eager: form.eager,
    deniedTools: splitList(form.deniedTools),
    allowedTools: splitList(form.allowedTools),
    oauth: null,
    projectPaths: [],
    icon: null,
  } as McpServerDraft
}

// ── Dialog ───────────────────────────────────────────────────────

export default function McpServerEditDialog({
  open,
  initial,
  onClose,
  onSaved,
}: {
  open: boolean
  initial: McpServerSummary | null
  onClose: () => void
  onSaved: () => void
}) {
  const { t } = useTranslation()
  const [form, setForm] = useState<FormState>(initialFromSummary(initial))
  const [saving, setSaving] = useState(false)

  useEffect(() => {
    setForm(initialFromSummary(initial))
  }, [initial])

  const isEditing = !!initial
  const title = isEditing
    ? t("settings.mcp.editTitle", { name: initial?.name ?? "" })
    : t("settings.mcp.addTitle")

  const nameInvalid = useMemo(() => {
    const n = form.name.trim()
    if (!n) return true
    return !/^[a-z0-9_-]{1,32}$/.test(n)
  }, [form.name])

  const autoApproveBlocked = form.autoApprove && form.trustLevel === "untrusted"

  const handleSave = async () => {
    if (nameInvalid) {
      toast.error(t("settings.mcp.invalidName"))
      return
    }
    if (autoApproveBlocked) {
      toast.error(t("settings.mcp.autoApproveNeedsTrust"))
      return
    }
    const draft = formToDraft(form)
    setSaving(true)
    try {
      if (isEditing && initial) {
        await updateServer(initial.id, draft)
      } else {
        await addServer(draft)
      }
      toast.success(t("settings.mcp.saved"))
      onSaved()
    } catch (e) {
      toast.error(String(e))
    } finally {
      setSaving(false)
    }
  }

  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-2xl max-h-[90vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
        </DialogHeader>

        <div className="space-y-4">
          {/* Name */}
          <div className="space-y-1.5">
            <Label>{t("settings.mcp.nameLabel")}</Label>
            <Input
              value={form.name}
              onChange={(e) => setForm({ ...form, name: e.target.value })}
              placeholder="memory"
              disabled={isEditing}
            />
            {nameInvalid && form.name.length > 0 && (
              <p className="text-xs text-destructive">
                {t("settings.mcp.invalidName")}
              </p>
            )}
            {isEditing && (
              <p className="text-xs text-muted-foreground">
                {t("settings.mcp.nameImmutable")}
              </p>
            )}
          </div>

          {/* Enabled */}
          <div className="flex items-center justify-between">
            <Label>{t("settings.mcp.enabledLabel")}</Label>
            <Switch
              checked={form.enabled}
              onCheckedChange={(v) => setForm({ ...form, enabled: v })}
            />
          </div>

          {/* Transport kind */}
          <div className="space-y-1.5">
            <Label>{t("settings.mcp.transportLabel")}</Label>
            <Select
              value={form.kind}
              onValueChange={(v) =>
                setForm({ ...form, kind: v as McpTransportKind })
              }
            >
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="stdio">stdio ({t("settings.mcp.transportStdioDesc")})</SelectItem>
                <SelectItem value="streamableHttp">Streamable HTTP</SelectItem>
                <SelectItem value="sse">SSE ({t("settings.mcp.transportLegacy")})</SelectItem>
                <SelectItem value="websocket">WebSocket</SelectItem>
              </SelectContent>
            </Select>
            {form.kind !== "stdio" && (
              <p className="text-xs text-muted-foreground">
                {t("settings.mcp.remoteTransportPhase4")}
              </p>
            )}
          </div>

          {/* Kind-specific fields */}
          {form.kind === "stdio" ? (
            <>
              <div className="space-y-1.5">
                <Label>{t("settings.mcp.commandLabel")}</Label>
                <Input
                  value={form.command}
                  onChange={(e) => setForm({ ...form, command: e.target.value })}
                  placeholder="npx"
                />
              </div>
              <div className="space-y-1.5">
                <Label>{t("settings.mcp.argsLabel")}</Label>
                <Textarea
                  value={form.args}
                  onChange={(e) => setForm({ ...form, args: e.target.value })}
                  placeholder="-y&#10;@modelcontextprotocol/server-memory"
                  rows={3}
                  className="font-mono text-sm"
                />
                <p className="text-xs text-muted-foreground">
                  {t("settings.mcp.argsHint")}
                </p>
              </div>
              <div className="space-y-1.5">
                <Label>{t("settings.mcp.cwdLabel")}</Label>
                <Input
                  value={form.cwd}
                  onChange={(e) => setForm({ ...form, cwd: e.target.value })}
                  placeholder={t("settings.mcp.cwdPlaceholder")}
                />
              </div>
              <KvEditor
                label={t("settings.mcp.envLabel")}
                entries={form.env}
                onChange={(env) => setForm({ ...form, env })}
                keyPlaceholder="API_KEY"
                valuePlaceholder="${MY_SECRET}"
                hint={t("settings.mcp.envHint")}
              />
            </>
          ) : (
            <>
              <div className="space-y-1.5">
                <Label>{t("settings.mcp.urlLabel")}</Label>
                <Input
                  value={form.url}
                  onChange={(e) => setForm({ ...form, url: e.target.value })}
                  placeholder="https://example.com/mcp"
                />
              </div>
              <KvEditor
                label={t("settings.mcp.headersLabel")}
                entries={form.headers}
                onChange={(headers) => setForm({ ...form, headers })}
                keyPlaceholder="Authorization"
                valuePlaceholder="Bearer ${TOKEN}"
              />
            </>
          )}

          {/* Description */}
          <div className="space-y-1.5">
            <Label>{t("settings.mcp.descriptionLabel")}</Label>
            <Input
              value={form.description}
              onChange={(e) => setForm({ ...form, description: e.target.value })}
              placeholder={t("settings.mcp.descriptionPlaceholder")}
            />
          </div>

          {/* Advanced — collapsible could come later */}
          <div className="pt-2 border-t border-border">
            <p className="text-xs font-semibold text-muted-foreground mb-3">
              {t("settings.mcp.advanced")}
            </p>
            <div className="grid grid-cols-3 gap-3">
              <TimeoutInput
                label={t("settings.mcp.connectTimeout")}
                value={form.connectTimeoutSecs}
                onChange={(v) => setForm({ ...form, connectTimeoutSecs: v })}
              />
              <TimeoutInput
                label={t("settings.mcp.callTimeout")}
                value={form.callTimeoutSecs}
                onChange={(v) => setForm({ ...form, callTimeoutSecs: v })}
              />
              <TimeoutInput
                label={t("settings.mcp.concurrency")}
                value={form.maxConcurrentCalls}
                onChange={(v) => setForm({ ...form, maxConcurrentCalls: v })}
              />
            </div>

            <div className="grid grid-cols-2 gap-3 mt-3">
              <div className="space-y-1">
                <Label>{t("settings.mcp.trustLevel")}</Label>
                <Select
                  value={form.trustLevel}
                  onValueChange={(v) =>
                    setForm({ ...form, trustLevel: v as McpTrustLevel })
                  }
                >
                  <SelectTrigger>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="untrusted">
                      {t("settings.mcp.trustUntrusted")}
                    </SelectItem>
                    <SelectItem value="trusted">
                      {t("settings.mcp.trustTrusted")}
                    </SelectItem>
                  </SelectContent>
                </Select>
              </div>
              <div className="flex flex-col gap-2 pt-5">
                <label className="flex items-center gap-2 text-sm">
                  <Switch
                    checked={form.autoApprove}
                    onCheckedChange={(v) =>
                      setForm({ ...form, autoApprove: v })
                    }
                  />
                  {t("settings.mcp.autoApproveLabel")}
                </label>
                <label className="flex items-center gap-2 text-sm">
                  <Switch
                    checked={form.eager}
                    onCheckedChange={(v) => setForm({ ...form, eager: v })}
                  />
                  {t("settings.mcp.eagerLabel")}
                </label>
              </div>
            </div>

            {autoApproveBlocked && (
              <p className="text-xs text-destructive mt-2">
                {t("settings.mcp.autoApproveNeedsTrust")}
              </p>
            )}

            <div className="grid grid-cols-2 gap-3 mt-3">
              <div className="space-y-1">
                <Label>{t("settings.mcp.allowedTools")}</Label>
                <Textarea
                  value={form.allowedTools}
                  onChange={(e) =>
                    setForm({ ...form, allowedTools: e.target.value })
                  }
                  placeholder={t("settings.mcp.toolListPlaceholder")}
                  rows={2}
                  className="font-mono text-xs"
                />
                <p className="text-xs text-muted-foreground">
                  {t("settings.mcp.allowedToolsHint")}
                </p>
              </div>
              <div className="space-y-1">
                <Label>{t("settings.mcp.deniedTools")}</Label>
                <Textarea
                  value={form.deniedTools}
                  onChange={(e) =>
                    setForm({ ...form, deniedTools: e.target.value })
                  }
                  placeholder={t("settings.mcp.toolListPlaceholder")}
                  rows={2}
                  className="font-mono text-xs"
                />
              </div>
            </div>
          </div>
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={onClose} disabled={saving}>
            {t("common.cancel")}
          </Button>
          <Button onClick={handleSave} disabled={saving || nameInvalid}>
            {saving ? (
              <>
                <Loader2 className="h-4 w-4 animate-spin mr-2" />
                {t("common.saving")}
              </>
            ) : (
              t("common.save")
            )}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

// ── KV editor ────────────────────────────────────────────────────

function KvEditor({
  label,
  entries,
  onChange,
  keyPlaceholder,
  valuePlaceholder,
  hint,
}: {
  label: string
  entries: Array<[string, string]>
  onChange: (next: Array<[string, string]>) => void
  keyPlaceholder?: string
  valuePlaceholder?: string
  hint?: string
}) {
  return (
    <div className="space-y-1.5">
      <Label>{label}</Label>
      <div className="space-y-1.5">
        {entries.map(([k, v], idx) => (
          <div key={idx} className="flex gap-1.5">
            <Input
              value={k}
              onChange={(e) => {
                const next = [...entries]
                next[idx] = [e.target.value, v]
                onChange(next)
              }}
              placeholder={keyPlaceholder}
              className="font-mono text-sm"
            />
            <Input
              value={v}
              onChange={(e) => {
                const next = [...entries]
                next[idx] = [k, e.target.value]
                onChange(next)
              }}
              placeholder={valuePlaceholder}
              className="font-mono text-sm"
            />
            <Button
              variant="ghost"
              size="sm"
              onClick={() => {
                const next = entries.filter((_, i) => i !== idx)
                onChange(next)
              }}
              className="h-9 w-9 p-0 shrink-0"
            >
              <X className="h-3.5 w-3.5" />
            </Button>
          </div>
        ))}
        <Button
          variant="outline"
          size="sm"
          onClick={() => onChange([...entries, ["", ""]])}
          className="gap-1.5 h-7"
        >
          <Plus className="h-3 w-3" />
          {entries.length === 0 ? "Add" : "Add more"}
        </Button>
      </div>
      {hint && <p className="text-xs text-muted-foreground">{hint}</p>}
    </div>
  )
}

function TimeoutInput({
  label,
  value,
  onChange,
}: {
  label: string
  value: string
  onChange: (v: string) => void
}) {
  return (
    <div className="space-y-1">
      <Label className="text-xs">{label}</Label>
      <Input
        type="number"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        min={1}
        className="h-9"
      />
    </div>
  )
}
