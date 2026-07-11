import { useState, useEffect, useCallback, useMemo } from "react"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import { Switch } from "@/components/ui/switch"
import { Input } from "@/components/ui/input"
import { Button } from "@/components/ui/button"
import { Textarea } from "@/components/ui/textarea"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { ChevronDown, ChevronRight, Loader2, Check, Plus, Trash2, Webhook } from "lucide-react"
import { cn } from "@/lib/utils"
import { logger } from "@/lib/logger"

// ── Types (mirror ha-core HooksConfig / HookHandlerConfig JSON) ──────────────

type HandlerType = "command" | "http" | "mcp_tool" | "prompt" | "agent"
type Handler = { type: HandlerType } & Record<string, unknown>
interface MatcherGroup {
  matcher?: string
  hooks: Handler[]
}
type HooksMap = Record<string, MatcherGroup[]>
interface HooksSettings {
  disableAllHooks: boolean
  allowProjectScope: boolean
  hooks: HooksMap
}

// The 26 events that actually fire. Protocol-reserved TeammateIdle and
// InstructionsLoaded are omitted because Hope Agent never dispatches them.
const FIREABLE_EVENTS: string[] = [
  "SessionStart",
  "SessionEnd",
  "UserPromptSubmit",
  "UserPromptExpansion",
  "PreToolUse",
  "PostToolUse",
  "PostToolUseFailure",
  "PostToolBatch",
  "PermissionRequest",
  "PermissionDenied",
  "Stop",
  "StopFailure",
  "PreCompact",
  "PostCompact",
  "Notification",
  "SubagentStart",
  "SubagentStop",
  "TaskCreated",
  "TaskCompleted",
  "ConfigChange",
  "CwdChanged",
  "FileChanged",
  "WorktreeCreate",
  "WorktreeRemove",
  "Elicitation",
  "ElicitationResult",
]

const HANDLER_TYPES: HandlerType[] = ["command", "http", "mcp_tool", "prompt", "agent"]

// Blocking events are awaited inline before the turn proceeds, so a slow
// handler (LLM side-query / sub-agent) stalls every fire. PreToolUse is the
// worst (once per tool call). Warn when these combine.
const BLOCKING_EVENTS = new Set(["PreToolUse", "UserPromptSubmit", "PreCompact", "WorktreeCreate"])
const SLOW_HANDLERS = new Set<HandlerType>(["prompt", "agent"])

type FieldKind = "text" | "textarea" | "number" | "switch" | "csv" | "json" | "shell"
interface FieldDef {
  key: string
  label: string
  kind: FieldKind
}

// Type-specific fields. Labels stay as the literal JSON keys (technical config
// terms the user edits) — only the panel chrome is translated.
const FIELDS_BY_TYPE: Record<HandlerType, FieldDef[]> = {
  command: [
    { key: "command", label: "command", kind: "textarea" },
    { key: "shell", label: "shell", kind: "shell" },
    { key: "async", label: "async", kind: "switch" },
    { key: "asyncRewake", label: "asyncRewake (inject on exit 2)", kind: "switch" },
  ],
  http: [
    { key: "url", label: "url", kind: "text" },
    { key: "headers", label: "headers (JSON)", kind: "json" },
    { key: "allowedEnvVars", label: "allowedEnvVars (a, b)", kind: "csv" },
  ],
  mcp_tool: [
    { key: "server", label: "server", kind: "text" },
    { key: "tool", label: "tool", kind: "text" },
    { key: "input", label: "input (JSON)", kind: "json" },
  ],
  prompt: [
    { key: "prompt", label: "prompt", kind: "textarea" },
    { key: "model", label: "model (provider:model)", kind: "text" },
  ],
  agent: [
    { key: "prompt", label: "prompt", kind: "textarea" },
    { key: "agent", label: "agent id", kind: "text" },
    { key: "allowedTools", label: "allowedTools (a, b)", kind: "csv" },
    { key: "async", label: "async", kind: "switch" },
  ],
}

// Shared across every handler type (rendered under "advanced").
const COMMON_FIELDS: FieldDef[] = [
  { key: "timeout", label: "timeout (s)", kind: "number" },
  { key: "if", label: "if (e.g. exec(rm *))", kind: "text" },
  { key: "statusMessage", label: "statusMessage", kind: "text" },
  { key: "once", label: "once", kind: "switch" },
]

function defaultHandler(type: HandlerType): Handler {
  switch (type) {
    case "command":
      return { type, command: "" }
    case "http":
      return { type, url: "" }
    case "mcp_tool":
      return { type, server: "", tool: "" }
    case "prompt":
      return { type, prompt: "" }
    case "agent":
      return { type, prompt: "" }
  }
}

// csv <-> string[]
function csvToArray(s: string): string[] {
  return s
    .split(",")
    .map((x) => x.trim())
    .filter(Boolean)
}

// ── Field renderers ─────────────────────────────────────────────────────────

/** JSON object field with a local text buffer so invalid mid-edit text doesn't
 * blow away the value; commits the parsed object only when it parses. */
function JsonField({ value, onChange }: { value: unknown; onChange: (v: unknown) => void }) {
  const { t } = useTranslation()
  const [text, setText] = useState(() => (value == null ? "" : JSON.stringify(value, null, 2)))
  const [err, setErr] = useState(false)
  return (
    <div className="space-y-1">
      <Textarea
        value={text}
        rows={2}
        className={cn("font-mono text-xs", err && "border-destructive")}
        onChange={(e) => {
          const next = e.target.value
          setText(next)
          if (next.trim() === "") {
            setErr(false)
            onChange(undefined)
            return
          }
          try {
            onChange(JSON.parse(next))
            setErr(false)
          } catch {
            setErr(true)
          }
        }}
      />
      {err && <p className="text-[11px] text-destructive">{t("settings.hooks.invalidJson")}</p>}
    </div>
  )
}

function FieldInput({
  def,
  value,
  onChange,
}: {
  def: FieldDef
  value: unknown
  onChange: (v: unknown) => void
}) {
  switch (def.kind) {
    case "switch":
      return <Switch checked={value === true} onCheckedChange={(c) => onChange(c || undefined)} />
    case "number":
      return (
        <Input
          type="number"
          value={value == null ? "" : String(value)}
          onChange={(e) => onChange(e.target.value === "" ? undefined : Number(e.target.value))}
          className="h-8"
        />
      )
    case "textarea":
      return (
        <Textarea
          value={typeof value === "string" ? value : ""}
          onChange={(e) => onChange(e.target.value || undefined)}
          rows={2}
          className="font-mono text-xs"
        />
      )
    case "csv":
      return (
        <Input
          value={Array.isArray(value) ? (value as string[]).join(", ") : ""}
          onChange={(e) => {
            const arr = csvToArray(e.target.value)
            onChange(arr.length ? arr : undefined)
          }}
          className="h-8"
        />
      )
    case "json":
      return <JsonField value={value} onChange={onChange} />
    case "shell":
      return (
        <Select
          value={typeof value === "string" && value ? value : "__default"}
          onValueChange={(v) => onChange(v === "__default" ? undefined : v)}
        >
          <SelectTrigger className="h-8">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="__default">default</SelectItem>
            <SelectItem value="bash">bash</SelectItem>
            <SelectItem value="powershell">powershell</SelectItem>
          </SelectContent>
        </Select>
      )
    default:
      return (
        <Input
          value={typeof value === "string" ? value : ""}
          onChange={(e) => onChange(e.target.value || undefined)}
          className="h-8"
        />
      )
  }
}

/** One labeled field row. */
function FieldRow({
  def,
  value,
  onChange,
}: {
  def: FieldDef
  value: unknown
  onChange: (v: unknown) => void
}) {
  const inline = def.kind === "switch"
  return (
    <div className={cn("gap-1", inline ? "flex items-center justify-between" : "flex flex-col")}>
      <label className="text-xs font-mono text-muted-foreground">{def.label}</label>
      <FieldInput def={def} value={value} onChange={onChange} />
    </div>
  )
}

// ── One handler / one matcher group ─────────────────────────────────────────

function HandlerCard({
  handler,
  event,
  onChange,
  onRemove,
}: {
  handler: Handler
  event: string
  onChange: (h: Handler) => void
  onRemove: () => void
}) {
  const { t } = useTranslation()
  const [adv, setAdv] = useState(false)
  const slowOnHotPath = BLOCKING_EVENTS.has(event) && SLOW_HANDLERS.has(handler.type)
  const setField = (key: string, v: unknown) => {
    const next: Handler = { ...handler }
    if (v === undefined) delete next[key]
    else next[key] = v
    onChange(next)
  }
  return (
    <div className="space-y-2 rounded-md border border-border/60 bg-muted/20 p-2">
      <div className="flex items-center gap-2">
        <Select value={handler.type} onValueChange={(v) => onChange(defaultHandler(v as HandlerType))}>
          <SelectTrigger className="h-8 w-32">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {HANDLER_TYPES.map((ht) => (
              <SelectItem key={ht} value={ht}>
                {ht}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <div className="flex-1" />
        <Button
          variant="ghost"
          size="icon"
          className="h-7 w-7 text-muted-foreground hover:text-destructive"
          onClick={onRemove}
        >
          <Trash2 className="h-3.5 w-3.5" />
        </Button>
      </div>
      {slowOnHotPath && (
        <p className="rounded bg-amber-500/10 px-2 py-1 text-[11px] text-amber-600 dark:text-amber-500">
          {t("settings.hooks.slowHandlerWarning")}
        </p>
      )}
      {FIELDS_BY_TYPE[handler.type].map((def) => (
        <FieldRow key={def.key} def={def} value={handler[def.key]} onChange={(v) => setField(def.key, v)} />
      ))}
      <button
        type="button"
        onClick={() => setAdv((x) => !x)}
        className="flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground"
      >
        {adv ? <ChevronDown className="h-3 w-3" /> : <ChevronRight className="h-3 w-3" />}
        {t("settings.hooks.advanced")}
      </button>
      {adv && (
        <div className="space-y-2 border-l border-border/40 pl-2">
          {COMMON_FIELDS.map((def) => (
            <FieldRow key={def.key} def={def} value={handler[def.key]} onChange={(v) => setField(def.key, v)} />
          ))}
        </div>
      )}
    </div>
  )
}

function GroupCard({
  group,
  event,
  onChange,
  onRemove,
}: {
  group: MatcherGroup
  event: string
  onChange: (g: MatcherGroup) => void
  onRemove: () => void
}) {
  const { t } = useTranslation()
  const setHandler = (i: number, h: Handler) => {
    const hooks = group.hooks.slice()
    hooks[i] = h
    onChange({ ...group, hooks })
  }
  const removeHandler = (i: number) => {
    const hooks = group.hooks.slice()
    hooks.splice(i, 1)
    onChange({ ...group, hooks })
  }
  return (
    <div className="space-y-2 rounded-md border border-border/60 p-2">
      <div className="flex items-center gap-2">
        <label className="shrink-0 font-mono text-xs text-muted-foreground">
          {t("settings.hooks.matcher")}
        </label>
        <Input
          value={group.matcher ?? ""}
          placeholder={t("settings.hooks.matcherPlaceholder")}
          onChange={(e) => onChange({ ...group, matcher: e.target.value || undefined })}
          className="h-8"
        />
        <Button
          variant="ghost"
          size="icon"
          className="h-7 w-7 text-muted-foreground hover:text-destructive"
          onClick={onRemove}
        >
          <Trash2 className="h-3.5 w-3.5" />
        </Button>
      </div>
      {group.hooks.map((h, i) => (
        <HandlerCard
          key={i}
          handler={h}
          event={event}
          onChange={(nh) => setHandler(i, nh)}
          onRemove={() => removeHandler(i)}
        />
      ))}
      <Button
        variant="outline"
        size="sm"
        className="h-7 text-xs"
        onClick={() => onChange({ ...group, hooks: [...group.hooks, defaultHandler("command")] })}
      >
        <Plus className="mr-1 h-3 w-3" />
        {t("settings.hooks.addHandler")}
      </Button>
    </div>
  )
}

// ── Panel ────────────────────────────────────────────────────────────────────

export default function HooksPanel() {
  const { t } = useTranslation()
  const [settings, setSettings] = useState<HooksSettings | null>(null)
  const [savedJson, setSavedJson] = useState("")
  const [saving, setSaving] = useState(false)
  const [saveStatus, setSaveStatus] = useState<"idle" | "saved" | "failed">("idle")
  const [collapsed, setCollapsed] = useState<Record<string, boolean>>({})
  const [pickEvent, setPickEvent] = useState("")

  useEffect(() => {
    getTransport()
      .call<HooksSettings>("get_hooks_config")
      .then((s) => {
        const norm: HooksSettings = {
          disableAllHooks: !!s.disableAllHooks,
          allowProjectScope: !!s.allowProjectScope,
          hooks: s.hooks ?? {},
        }
        setSettings(norm)
        setSavedJson(JSON.stringify(norm))
      })
      .catch((e) => logger.error("settings", "HooksPanel::load", "Failed to load hooks config", e))
  }, [])

  const dirty = useMemo(
    () => settings != null && JSON.stringify(settings) !== savedJson,
    [settings, savedJson],
  )

  const save = useCallback(async () => {
    if (!settings) return
    setSaving(true)
    try {
      await getTransport().call("save_hooks_config", { config: settings })
      setSavedJson(JSON.stringify(settings))
      setSaveStatus("saved")
      setTimeout(() => setSaveStatus("idle"), 2000)
    } catch (e) {
      logger.error("settings", "HooksPanel::save", "Failed to save hooks config", e)
      setSaveStatus("failed")
      setTimeout(() => setSaveStatus("idle"), 2000)
    } finally {
      setSaving(false)
    }
  }, [settings])

  if (!settings) {
    return (
      <div className="flex items-center gap-2 p-4 text-sm text-muted-foreground">
        <Loader2 className="h-4 w-4 animate-spin" /> {t("common.loading")}
      </div>
    )
  }

  const setHooks = (hooks: HooksMap) => setSettings({ ...settings, hooks })
  const eventsWithHooks = FIREABLE_EVENTS.filter((ev) => (settings.hooks[ev]?.length ?? 0) > 0)

  const addGroup = (ev: string) => {
    const groups = settings.hooks[ev]?.slice() ?? []
    groups.push({ hooks: [defaultHandler("command")] })
    setHooks({ ...settings.hooks, [ev]: groups })
    setCollapsed((c) => ({ ...c, [ev]: false }))
  }
  const setGroup = (ev: string, i: number, g: MatcherGroup) => {
    const groups = settings.hooks[ev].slice()
    groups[i] = g
    setHooks({ ...settings.hooks, [ev]: groups })
  }
  const removeGroup = (ev: string, i: number) => {
    const groups = settings.hooks[ev].slice()
    groups.splice(i, 1)
    const next = { ...settings.hooks }
    if (groups.length) next[ev] = groups
    else delete next[ev]
    setHooks(next)
  }

  return (
    <div className="mx-auto max-w-4xl space-y-4 p-1">
      <div className="flex items-start justify-between gap-4">
        <div className="space-y-1">
          <h2 className="flex items-center gap-2 text-lg font-semibold">
            <Webhook className="h-5 w-5" /> {t("settings.hooks.title")}
          </h2>
          <p className="text-sm text-muted-foreground">{t("settings.hooks.intro")}</p>
        </div>
        <Button
          size="sm"
          onClick={save}
          disabled={(!dirty && saveStatus === "idle") || saving}
          className={cn(
            saveStatus === "saved" && "bg-green-500/10 text-green-600 hover:bg-green-500/20",
            saveStatus === "failed" && "bg-destructive/10 text-destructive hover:bg-destructive/20",
          )}
        >
          {saving ? (
            <span className="flex items-center gap-1.5">
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
              {t("common.saving")}
            </span>
          ) : saveStatus === "saved" ? (
            <span className="flex items-center gap-1.5">
              <Check className="h-3.5 w-3.5" />
              {t("common.saved")}
            </span>
          ) : saveStatus === "failed" ? (
            t("common.saveFailed")
          ) : (
            t("common.save")
          )}
        </Button>
      </div>

      <div className="flex items-center justify-between rounded-md border border-border/60 p-3">
        <div>
          <div className="text-sm font-medium">{t("settings.hooks.disableAll")}</div>
          <div className="text-xs text-muted-foreground">{t("settings.hooks.disableAllDesc")}</div>
        </div>
        <Switch
          checked={settings.disableAllHooks}
          onCheckedChange={(c) => setSettings({ ...settings, disableAllHooks: c })}
        />
      </div>

      <div className="flex items-center justify-between rounded-md border border-border/60 p-3">
        <div>
          <div className="text-sm font-medium">{t("settings.hooks.allowProjectScope")}</div>
          <div className="text-xs text-muted-foreground">
            {t("settings.hooks.allowProjectScopeDesc")}
          </div>
        </div>
        <Switch
          checked={settings.allowProjectScope}
          onCheckedChange={(c) => setSettings({ ...settings, allowProjectScope: c })}
        />
      </div>

      <p className="rounded-md bg-muted/40 p-2 text-xs text-muted-foreground">
        {t("settings.hooks.scopeNote")}
      </p>

      <div className="flex items-center gap-2">
        <Select value={pickEvent} onValueChange={setPickEvent}>
          <SelectTrigger className="h-9 flex-1">
            <SelectValue placeholder={t("settings.hooks.pickEvent")} />
          </SelectTrigger>
          <SelectContent>
            {FIREABLE_EVENTS.map((ev) => (
              <SelectItem key={ev} value={ev}>
                {ev}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <Button
          size="sm"
          disabled={!pickEvent}
          onClick={() => {
            if (pickEvent) addGroup(pickEvent)
          }}
        >
          <Plus className="mr-1 h-4 w-4" />
          {t("settings.hooks.addHook")}
        </Button>
      </div>

      {eventsWithHooks.length === 0 ? (
        <p className="py-6 text-center text-sm text-muted-foreground">{t("settings.hooks.empty")}</p>
      ) : (
        eventsWithHooks.map((ev) => {
          const groups = settings.hooks[ev]
          const isCollapsed = collapsed[ev]
          return (
            <div key={ev} className="space-y-2 rounded-lg border border-border p-3">
              <button
                type="button"
                onClick={() => setCollapsed((c) => ({ ...c, [ev]: !c[ev] }))}
                className="flex w-full items-center gap-2 text-sm font-medium"
              >
                {isCollapsed ? (
                  <ChevronRight className="h-4 w-4" />
                ) : (
                  <ChevronDown className="h-4 w-4" />
                )}
                {ev}
                <span className="text-xs text-muted-foreground">({groups.length})</span>
              </button>
              {!isCollapsed && (
                <div className="space-y-2">
                  {groups.map((g, i) => (
                    <GroupCard
                      key={i}
                      group={g}
                      event={ev}
                      onChange={(ng) => setGroup(ev, i, ng)}
                      onRemove={() => removeGroup(ev, i)}
                    />
                  ))}
                  <Button
                    variant="outline"
                    size="sm"
                    className="h-7 text-xs"
                    onClick={() => addGroup(ev)}
                  >
                    <Plus className="mr-1 h-3 w-3" />
                    {t("settings.hooks.addGroup")}
                  </Button>
                </div>
              )}
            </div>
          )
        })
      )}
    </div>
  )
}
