import { useState } from "react"
import { useTranslation } from "react-i18next"
import { useSortable } from "@dnd-kit/sortable"
import { CSS } from "@dnd-kit/utilities"
import type { DraggableAttributes, DraggableSyntheticListeners } from "@dnd-kit/core"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import {
  CheckCircle2,
  Clock,
  GripVertical,
  Image,
  Info,
  Loader2,
  Play,
  Trash2,
  Type,
  Video,
  X,
  XCircle,
} from "lucide-react"
import type { ModelConfig } from "./types"

interface ModelTestData {
  success?: boolean
  message?: string
  latencyMs?: number
  reply?: string
  request?: unknown
  response?: unknown
  status?: number | string
  model?: string
}

// ── SortableModelEditor ──────────────────────────────────────────

export function SortableModelEditor({
  sortableId,
  model,
  onChange,
  onRemove,
  onTest,
}: {
  sortableId: string
  model: ModelConfig
  onChange: (m: ModelConfig) => void
  onRemove: () => void
  onTest?: (modelId: string) => Promise<string>
}) {
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({
    id: sortableId,
  })

  const style = {
    transform: CSS.Transform.toString(transform),
    transition,
    opacity: isDragging ? 0.4 : 1,
    zIndex: isDragging ? 50 : undefined,
  }

  return (
    <div ref={setNodeRef} style={style}>
      <ModelEditor
        model={model}
        onChange={onChange}
        onRemove={onRemove}
        onTest={onTest}
        dragListeners={listeners}
        dragAttributes={attributes}
      />
    </div>
  )
}

// ── ModelEditor ──────────────────────────────────────────────────

export function ModelEditor({
  model,
  onChange,
  onRemove,
  onTest,
  dragListeners,
  dragAttributes,
}: {
  model: ModelConfig
  onChange: (m: ModelConfig) => void
  onRemove: () => void
  onTest?: (modelId: string) => Promise<string>
  dragListeners?: DraggableSyntheticListeners
  dragAttributes?: DraggableAttributes
}) {
  const { t } = useTranslation()
  const inputTypes = ["text", "image", "video"]
  const [testLoading, setTestLoading] = useState(false)
  const [testResult, setTestResult] = useState<{
    ok: boolean
    data: ModelTestData
  } | null>(null)
  const [logExpanded, setLogExpanded] = useState(false)

  function toggleInput(type: string) {
    const current = model.inputTypes
    if (current.includes(type)) {
      onChange({ ...model, inputTypes: current.filter((t) => t !== type) })
    } else {
      onChange({ ...model, inputTypes: [...current, type] })
    }
  }

  return (
    <div className="border border-border rounded-lg p-3.5 space-y-3 bg-secondary/60">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-1.5">
          {dragListeners && (
            <div
              className="cursor-grab active:cursor-grabbing text-muted-foreground/40 hover:text-muted-foreground/70 touch-none"
              {...dragAttributes}
              {...dragListeners}
            >
              <GripVertical className="h-3.5 w-3.5" />
            </div>
          )}
          <span className="text-[10px] font-medium text-muted-foreground/70 uppercase tracking-wider">
            {t("model.modelConfig")}
          </span>
        </div>
        <Button
          variant="ghost"
          size="icon"
          className="h-6 w-6 text-muted-foreground hover:text-red-400"
          onClick={onRemove}
        >
          <Trash2 className="h-3 w-3" />
        </Button>
      </div>

      <div className="grid grid-cols-2 gap-2.5">
        <div className="space-y-1">
          <label className="text-[10px] text-muted-foreground">{t("model.modelId")}</label>
          <Input
            value={model.id}
            onChange={(e) => onChange({ ...model, id: e.target.value })}
            placeholder="model-id"
            className="bg-background text-xs h-8"
          />
        </div>
        <div className="space-y-1">
          <label className="text-[10px] text-muted-foreground">{t("model.displayName")}</label>
          <Input
            value={model.name}
            onChange={(e) => onChange({ ...model, name: e.target.value })}
            placeholder="Model Name"
            className="bg-background text-xs h-8"
          />
        </div>
      </div>

      <div className="space-y-1.5">
        <label className="text-[10px] text-muted-foreground">
          {t("model.supportedInputTypes")}
        </label>
        <div className="flex gap-2">
          {inputTypes.map((type) => (
            <button
              key={type}
              onClick={() => toggleInput(type)}
              className={`px-2.5 py-1 text-[11px] rounded-md border transition-colors flex items-center gap-1.5 ${
                model.inputTypes.includes(type)
                  ? "border-primary bg-primary/10 text-primary"
                  : "border-border bg-background text-muted-foreground hover:border-primary/40"
              }`}
            >
              {type === "text" && <Type className="h-3 w-3" />}
              {type === "image" && <Image className="h-3 w-3" />}
              {type === "video" && <Video className="h-3 w-3" />}
              {type === "text"
                ? t("model.text")
                : type === "image"
                  ? t("model.image")
                  : t("model.video")}
            </button>
          ))}
        </div>
      </div>

      <div className="grid grid-cols-2 gap-2.5">
        <div className="space-y-1">
          <label className="text-[10px] text-muted-foreground">Context Window</label>
          <Input
            type="number"
            value={model.contextWindow}
            onChange={(e) =>
              onChange({
                ...model,
                contextWindow: parseInt(e.target.value) || 0,
              })
            }
            className="bg-background text-xs h-8"
          />
        </div>
        <div className="space-y-1">
          <label className="text-[10px] text-muted-foreground">Max Tokens</label>
          <Input
            type="number"
            value={model.maxTokens}
            onChange={(e) => onChange({ ...model, maxTokens: parseInt(e.target.value) || 0 })}
            className="bg-background text-xs h-8"
          />
        </div>
      </div>

      <div className="flex items-center justify-between">
        <label className="text-xs text-muted-foreground">{t("model.reasoning")}</label>
        <button
          onClick={() => onChange({ ...model, reasoning: !model.reasoning })}
          className={`w-9 h-5 rounded-full transition-colors relative ${
            model.reasoning ? "bg-primary" : "bg-secondary border border-border"
          }`}
        >
          <span
            className={`absolute top-0.5 w-4 h-4 rounded-full bg-white shadow transition-transform ${
              model.reasoning ? "left-[18px]" : "left-0.5"
            }`}
          />
        </button>
      </div>

      <div className="grid grid-cols-2 gap-2.5">
        <div className="space-y-1">
          <label className="text-[10px] text-muted-foreground">{t("model.inputCost")}</label>
          <Input
            type="number"
            step="0.01"
            value={model.costInput}
            onChange={(e) =>
              onChange({
                ...model,
                costInput: parseFloat(e.target.value) || 0,
              })
            }
            className="bg-background text-xs h-8"
          />
        </div>
        <div className="space-y-1">
          <label className="text-[10px] text-muted-foreground">{t("model.outputCost")}</label>
          <Input
            type="number"
            step="0.01"
            value={model.costOutput}
            onChange={(e) =>
              onChange({
                ...model,
                costOutput: parseFloat(e.target.value) || 0,
              })
            }
            className="bg-background text-xs h-8"
          />
        </div>
      </div>

      {/* Per-model test */}
      {onTest && model.id && (
        <div className="space-y-1.5 pt-1 border-t border-border/50">
          <div className="flex items-center gap-2">
            <button
              onClick={async () => {
                if (!onTest || !model.id) return
                setTestLoading(true)
                setTestResult(null)
                setLogExpanded(false)
                try {
                  const msg = await onTest(model.id)
                  const data = JSON.parse(msg)
                  setTestResult({ ok: data.success ?? true, data })
                } catch (e) {
                  try {
                    const data = JSON.parse(String(e))
                    setTestResult({ ok: false, data })
                  } catch {
                    setTestResult({ ok: false, data: { success: false, message: String(e) } })
                  }
                } finally {
                  setTestLoading(false)
                }
              }}
              disabled={testLoading}
              className="flex items-center gap-1 text-[10px] text-primary/70 hover:text-primary transition-colors disabled:opacity-50"
            >
              {testLoading ? (
                <Loader2 className="h-3 w-3 animate-spin" />
              ) : (
                <Play className="h-3 w-3 fill-current" />
              )}
              发送 "Hi" 测试
            </button>
            {testResult && (
              <span
                className={`flex items-center gap-1 text-[10px] ${testResult.ok ? "text-green-400" : "text-red-400"}`}
              >
                {testResult.ok ? (
                  <CheckCircle2 className="h-3 w-3" />
                ) : (
                  <XCircle className="h-3 w-3" />
                )}
                {testResult.data.message}
                {testResult.data.latencyMs != null && testResult.data.latencyMs > 0 && (
                  <span className="text-muted-foreground flex items-center gap-0.5">
                    <Clock className="h-2.5 w-2.5" />
                    {testResult.data.latencyMs}ms
                  </span>
                )}
                <button
                  onClick={() => setLogExpanded(!logExpanded)}
                  className="text-muted-foreground hover:text-foreground transition-colors ml-0.5"
                  title="查看完整日志"
                >
                  <Info className="h-3 w-3" />
                </button>
              </span>
            )}
          </div>
          {testResult?.ok && testResult.data.reply && (
            <div className="px-2.5 py-1.5 rounded-md bg-secondary/50 text-[10px] text-muted-foreground border border-border/50">
              <span className="text-[9px] font-medium text-foreground/60">AI 回复: </span>
              {testResult.data.reply}
            </div>
          )}
          {logExpanded &&
            testResult &&
            (() => {
              const d = testResult.data
              return (
                <div className="px-2.5 py-2 rounded-md bg-secondary/30 border border-border/50 overflow-hidden space-y-2">
                  <div className="flex items-center justify-between">
                    <span className="text-[9px] font-medium text-foreground/60">完整日志</span>
                    <button
                      onClick={() => setLogExpanded(false)}
                      className="text-muted-foreground hover:text-foreground"
                    >
                      <X className="h-2.5 w-2.5" />
                    </button>
                  </div>
                  {d.request != null && (
                    <div>
                      <span className="text-[9px] font-semibold text-blue-400">▸ 请求</span>
                      <pre className="text-[10px] text-muted-foreground whitespace-pre-wrap break-all max-h-32 overflow-y-auto font-mono mt-0.5 pl-2 border-l-2 border-blue-500/30">
                        {JSON.stringify(d.request, null, 2)}
                      </pre>
                    </div>
                  )}
                  <div>
                    <span
                      className={`text-[9px] font-semibold ${d.success ? "text-green-400" : "text-red-400"}`}
                    >
                      ▸ 响应 {d.status ? `(${d.status})` : ""}
                    </span>
                    <pre
                      className={`text-[10px] text-muted-foreground whitespace-pre-wrap break-all max-h-40 overflow-y-auto font-mono mt-0.5 pl-2 border-l-2 ${d.success ? "border-green-500/30" : "border-red-500/30"}`}
                    >
                      {d.response
                        ? JSON.stringify(d.response, null, 2)
                        : JSON.stringify(
                            {
                              success: d.success,
                              message: d.message,
                              model: d.model,
                              latencyMs: d.latencyMs,
                            },
                            null,
                            2,
                          )}
                    </pre>
                  </div>
                </div>
              )
            })()}
        </div>
      )}
    </div>
  )
}
