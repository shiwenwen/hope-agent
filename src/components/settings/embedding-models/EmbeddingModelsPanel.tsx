import { useCallback, useEffect, useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import {
  CheckCircle2,
  Loader2,
  Pencil,
  Plus,
  Star,
  Trash2,
  Wifi,
} from "lucide-react"
import { toast } from "sonner"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import LocalEmbeddingAssistantCard from "@/components/settings/memory-panel/LocalEmbeddingAssistantCard"
import {
  embeddingProviderLabel,
  type EmbeddingModelConfig,
  type EmbeddingModelTemplate,
  type EmbeddingModelTemplateModel,
  type EmbeddingProviderType,
  type MemoryEmbeddingSetDefaultResult,
  type MemoryEmbeddingState,
} from "@/types/embedding-models"

const PROVIDER_TYPES: EmbeddingProviderType[] = ["openai-compatible", "google"]
const CUSTOM_TEMPLATE_VALUE = "__custom_template__"
const CUSTOM_MODEL_VALUE = "__custom_model__"

function templateKey(template: EmbeddingModelTemplate): string {
  return `${template.providerType}::${template.baseUrl}::${template.name}`
}

function normalizeBaseUrl(url?: string | null): string {
  return (url ?? "").trim().replace(/\/+$/, "").toLowerCase()
}

function templateModels(template: EmbeddingModelTemplate): EmbeddingModelTemplateModel[] {
  return template.models?.length
    ? template.models
    : [
        {
          id: template.defaultModel,
          name: template.defaultModel,
          dimensions: template.defaultDimensions,
        },
      ]
}

function findMatchingTemplate(
  templates: EmbeddingModelTemplate[],
  config: EmbeddingModelConfig | null,
): EmbeddingModelTemplate | null {
  if (!config) return null
  return (
    templates.find(
      (template) =>
        template.providerType === config.providerType &&
        normalizeBaseUrl(template.baseUrl) === normalizeBaseUrl(config.apiBaseUrl),
    ) ?? null
  )
}

function findMatchingTemplateModel(
  template: EmbeddingModelTemplate | null,
  config: EmbeddingModelConfig | null,
): EmbeddingModelTemplateModel | null {
  if (!template || !config?.apiModel) return null
  return (
    templateModels(template).find(
      (model) =>
        model.id === config.apiModel &&
        (!config.apiDimensions || model.dimensions === config.apiDimensions),
    ) ?? null
  )
}

function configNameForTemplateModel(
  template: EmbeddingModelTemplate,
  model: EmbeddingModelTemplateModel,
): string {
  return `${template.name} · ${model.name} (${model.dimensions}d)`
}

function emptyConfig(
  template?: EmbeddingModelTemplate,
  selectedModel?: EmbeddingModelTemplateModel,
): EmbeddingModelConfig {
  if (template) {
    const model = selectedModel ?? templateModels(template)[0]
    return {
      id: "",
      name: configNameForTemplateModel(template, model),
      providerType: template.providerType,
      apiBaseUrl: template.baseUrl,
      apiKey: template.name === "Ollama" ? "ollama" : "",
      apiModel: model.id,
      apiDimensions: model.dimensions,
      source: template.name === "Ollama" ? "ollama" : "template",
    }
  }
  return {
    id: "",
    name: "",
    providerType: "openai-compatible",
    apiBaseUrl: "",
    apiKey: "",
    apiModel: "",
    apiDimensions: null,
    source: "custom",
  }
}

export default function EmbeddingModelsPanel() {
  const { t } = useTranslation()
  const [models, setModels] = useState<EmbeddingModelConfig[]>([])
  const [templates, setTemplates] = useState<EmbeddingModelTemplate[]>([])
  const [memoryState, setMemoryState] = useState<MemoryEmbeddingState>({
    selection: { enabled: false },
    currentModel: null,
    needsReembed: false,
  })
  const [loading, setLoading] = useState(false)
  const [saving, setSaving] = useState(false)
  const [testingId, setTestingId] = useState<string | null>(null)
  const [editing, setEditing] = useState<EmbeddingModelConfig | null>(null)
  const [pendingDefault, setPendingDefault] = useState<EmbeddingModelConfig | null>(null)
  const [pendingDelete, setPendingDelete] = useState<EmbeddingModelConfig | null>(null)

  const activeId = memoryState.selection.enabled ? memoryState.selection.modelConfigId : undefined

  const load = useCallback(async () => {
    setLoading(true)
    try {
      const [nextModels, nextTemplates, nextState] = await Promise.all([
        getTransport().call<EmbeddingModelConfig[]>("embedding_model_config_list"),
        getTransport().call<EmbeddingModelTemplate[]>("embedding_model_config_templates"),
        getTransport().call<MemoryEmbeddingState>("memory_embedding_get"),
      ])
      setModels(nextModels)
      setTemplates(nextTemplates)
      setMemoryState(nextState)
    } catch (e) {
      logger.error("settings", "EmbeddingModelsPanel::load", "Failed to load", e)
      toast.error(String(e))
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    void load()
  }, [load])

  const sortedModels = useMemo(
    () =>
      [...models].sort((a, b) => {
        if (a.id === activeId) return -1
        if (b.id === activeId) return 1
        return a.name.localeCompare(b.name)
      }),
    [activeId, models],
  )

  const selectedTemplate = useMemo(
    () => findMatchingTemplate(templates, editing),
    [editing, templates],
  )
  const selectedTemplateModels = useMemo(
    () => (selectedTemplate ? templateModels(selectedTemplate) : []),
    [selectedTemplate],
  )
  const selectedTemplateModel = useMemo(
    () => findMatchingTemplateModel(selectedTemplate, editing),
    [editing, selectedTemplate],
  )
  const selectedTemplateModelIndex = useMemo(
    () =>
      selectedTemplateModel
        ? selectedTemplateModels.findIndex(
            (model) =>
              model.id === selectedTemplateModel.id &&
              model.dimensions === selectedTemplateModel.dimensions,
          )
        : -1,
    [selectedTemplateModel, selectedTemplateModels],
  )
  const showCustomModelInput = !selectedTemplate || selectedTemplateModelIndex < 0

  function applyTemplate(template: EmbeddingModelTemplate | null) {
    if (!editing) return
    if (!template) {
      setEditing({ ...editing, source: editing.source === "ollama" ? "ollama" : "custom" })
      return
    }
    const next = emptyConfig(template)
    setEditing({
      ...editing,
      name: editing.id ? editing.name : next.name,
      providerType: next.providerType,
      apiBaseUrl: next.apiBaseUrl,
      apiKey: template.name === "Ollama" ? "ollama" : editing.apiKey,
      apiModel: next.apiModel,
      apiDimensions: next.apiDimensions,
      source: next.source,
    })
  }

  function applyTemplateModel(model: EmbeddingModelTemplateModel | null) {
    if (!editing || !selectedTemplate || !model) return
    setEditing({
      ...editing,
      name: editing.id ? editing.name : configNameForTemplateModel(selectedTemplate, model),
      apiModel: model.id,
      apiDimensions: model.dimensions,
      source: selectedTemplate.name === "Ollama" ? "ollama" : "template",
    })
  }

  async function saveEditing() {
    if (!editing) return
    setSaving(true)
    try {
      await getTransport().call<EmbeddingModelConfig>("embedding_model_config_save", {
        config: editing,
      })
      setEditing(null)
      await load()
      toast.success(t("common.saved"))
    } catch (e) {
      logger.error("settings", "EmbeddingModelsPanel::save", "Failed to save", e)
      toast.error(String(e))
    } finally {
      setSaving(false)
    }
  }

  async function testModel(model: EmbeddingModelConfig) {
    setTestingId(model.id || "__draft__")
    try {
      await getTransport().call("embedding_model_config_test", { config: model })
      toast.success(t("settings.embeddingModels.testOk"))
    } catch (e) {
      toast.error(String(e))
    } finally {
      setTestingId(null)
    }
  }

  async function confirmDefault() {
    if (!pendingDefault) return
    try {
      const result = await getTransport().call<MemoryEmbeddingSetDefaultResult>(
        "memory_embedding_set_default",
        { modelConfigId: pendingDefault.id, mode: "keep_existing" },
      )
      setMemoryState(result.state)
      await load()
      if (result.reembedError) {
        toast.warning(t("settings.embeddingModels.reembedFailed"))
      } else {
        toast.success(t("settings.embeddingModels.defaultSet"))
      }
    } catch (e) {
      toast.error(String(e))
    } finally {
      setPendingDefault(null)
    }
  }

  async function confirmDelete() {
    if (!pendingDelete) return
    try {
      await getTransport().call("embedding_model_config_delete", { id: pendingDelete.id })
      await load()
      toast.success(t("settings.embeddingModels.deleted"))
    } catch (e) {
      toast.error(String(e))
    } finally {
      setPendingDelete(null)
    }
  }

  function handleLocalEmbeddingActivated(result: MemoryEmbeddingSetDefaultResult) {
    setMemoryState(result.state)
    void load().then(() => {
      if (result.reembedError) {
        toast.warning(t("settings.embeddingModels.reembedFailed"))
      } else {
        toast.success(t("settings.localEmbedding.activated"))
      }
    })
  }

  return (
    <div className="flex-1 overflow-y-auto px-6 pb-6 pt-2">
      <div className="mb-4 flex flex-col gap-3 lg:flex-row lg:items-start lg:justify-between">
        <div>
          <h2 className="text-base font-semibold">{t("settings.embeddingModels.title")}</h2>
          <p className="mt-1 text-sm text-muted-foreground">
            {t("settings.embeddingModels.desc")}
          </p>
        </div>
        <Button onClick={() => setEditing(emptyConfig())}>
          <Plus className="mr-1.5 h-4 w-4" />
          {t("settings.embeddingModels.custom")}
        </Button>
      </div>

      <div className="mb-5">
        <LocalEmbeddingAssistantCard onActivated={handleLocalEmbeddingActivated} />
      </div>

      <div className="mb-5 flex flex-wrap gap-2">
        {templates.map((template) => (
          <Button
            key={`${template.name}-${template.defaultModel}`}
            variant="outline"
            size="sm"
            onClick={() => setEditing(emptyConfig(template))}
          >
            <Plus className="mr-1.5 h-3.5 w-3.5" />
            {template.name}
          </Button>
        ))}
      </div>

      {loading ? (
        <div className="flex h-32 items-center justify-center text-sm text-muted-foreground">
          <Loader2 className="mr-2 h-4 w-4 animate-spin" />
          {t("common.loading")}
        </div>
      ) : sortedModels.length === 0 ? (
        <div className="rounded-lg border border-dashed border-border bg-card/50 p-8 text-center text-sm text-muted-foreground">
          {t("settings.embeddingModels.empty")}
        </div>
      ) : (
        <div className="space-y-3">
          {sortedModels.map((model) => {
            const isActive = model.id === activeId
            const testing = testingId === model.id
            return (
              <div key={model.id} className="rounded-lg border border-border bg-card p-4">
                <div className="flex flex-col gap-3 lg:flex-row lg:items-start lg:justify-between">
                  <div className="min-w-0">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="truncate text-sm font-semibold">{model.name}</span>
                      <span className="rounded border border-border bg-secondary px-1.5 py-0.5 text-[10px] text-muted-foreground">
                        {embeddingProviderLabel(model)}
                      </span>
                      {isActive && (
                        <span className="rounded border border-emerald-500/25 bg-emerald-500/10 px-1.5 py-0.5 text-[10px] font-medium text-emerald-600 dark:text-emerald-400">
                          {t("settings.embeddingModels.memoryActive")}
                        </span>
                      )}
                    </div>
                    <div className="mt-1 text-xs text-muted-foreground">
                      {model.apiModel}
                      {model.apiDimensions ? ` · ${model.apiDimensions}d` : ""}
                      {model.apiBaseUrl ? ` · ${model.apiBaseUrl}` : ""}
                    </div>
                  </div>
                  <div className="flex flex-wrap items-center gap-2">
                    <Button variant="outline" size="sm" onClick={() => void testModel(model)}>
                      {testing ? (
                        <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />
                      ) : (
                        <Wifi className="mr-1.5 h-3.5 w-3.5" />
                      )}
                      {t("common.test")}
                    </Button>
                    {!isActive && (
                      <Button variant="outline" size="sm" onClick={() => setPendingDefault(model)}>
                        <Star className="mr-1.5 h-3.5 w-3.5" />
                        {t("settings.embeddingModels.setMemoryDefault")}
                      </Button>
                    )}
                    {isActive && (
                      <Button variant="secondary" size="sm" disabled>
                        <CheckCircle2 className="mr-1.5 h-3.5 w-3.5" />
                        {t("settings.embeddingModels.memoryActive")}
                      </Button>
                    )}
                    <Button variant="ghost" size="sm" onClick={() => setEditing(model)}>
                      <Pencil className="mr-1.5 h-3.5 w-3.5" />
                      {t("common.edit")}
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      className="text-destructive hover:text-destructive"
                      disabled={isActive}
                      onClick={() => setPendingDelete(model)}
                    >
                      <Trash2 className="mr-1.5 h-3.5 w-3.5" />
                      {t("common.delete")}
                    </Button>
                  </div>
                </div>
              </div>
            )
          })}
        </div>
      )}

      <Dialog open={!!editing} onOpenChange={(open) => !open && setEditing(null)}>
        <DialogContent className="sm:max-w-2xl">
          <DialogHeader>
            <DialogTitle>{t("settings.embeddingModels.editTitle")}</DialogTitle>
          </DialogHeader>
          {editing && (
            <div className="grid gap-4 py-2">
              <div className="grid gap-4 sm:grid-cols-2">
                <div className="grid gap-1.5">
                  <Label>{t("team.template")}</Label>
                  <Select
                    value={selectedTemplate ? templateKey(selectedTemplate) : CUSTOM_TEMPLATE_VALUE}
                    onValueChange={(value) => {
                      const template =
                        value === CUSTOM_TEMPLATE_VALUE
                          ? null
                          : templates.find((item) => templateKey(item) === value) ?? null
                      applyTemplate(template)
                    }}
                  >
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value={CUSTOM_TEMPLATE_VALUE}>{t("common.custom")}</SelectItem>
                      {templates.map((template) => (
                        <SelectItem key={templateKey(template)} value={templateKey(template)}>
                          {template.name}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
                <div className="grid gap-1.5">
                  <Label>{t("settings.embeddingModels.name")}</Label>
                  <Input
                    value={editing.name}
                    onChange={(e) => setEditing({ ...editing, name: e.target.value })}
                  />
                </div>
              </div>
              <div className="grid gap-1.5">
                <Label>{t("settings.embeddingModels.providerType")}</Label>
                <Select
                  value={editing.providerType}
                  onValueChange={(value) =>
                    setEditing({
                      ...editing,
                      providerType: value as EmbeddingProviderType,
                      source: editing.source === "ollama" ? "ollama" : "custom",
                    })
                  }
                >
                  <SelectTrigger>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {PROVIDER_TYPES.map((type) => (
                      <SelectItem key={type} value={type}>
                        {type === "google" ? "Google" : "OpenAI Compatible"}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
              <div className="grid gap-1.5">
                <Label>Base URL</Label>
                <Input
                  value={editing.apiBaseUrl ?? ""}
                  onChange={(e) =>
                    setEditing({
                      ...editing,
                      apiBaseUrl: e.target.value,
                      source: editing.source === "ollama" ? "ollama" : "custom",
                    })
                  }
                  placeholder="https://api.openai.com"
                />
              </div>
              <div className="grid gap-1.5">
                <Label>API Key</Label>
                <Input
                  type="password"
                  value={editing.apiKey ?? ""}
                  onChange={(e) => setEditing({ ...editing, apiKey: e.target.value })}
                  placeholder="sk-..."
                />
              </div>
              {selectedTemplate && (
                <div className="grid gap-1.5">
                  <Label>{t("settings.memoryModel")}</Label>
                  <Select
                    value={
                      selectedTemplateModelIndex >= 0
                        ? String(selectedTemplateModelIndex)
                        : CUSTOM_MODEL_VALUE
                    }
                    onValueChange={(value) => {
                      if (value === CUSTOM_MODEL_VALUE) {
                        setEditing({
                          ...editing,
                          apiModel: "",
                          apiDimensions: null,
                          source: editing.source === "ollama" ? "ollama" : "custom",
                        })
                        return
                      }
                      applyTemplateModel(selectedTemplateModels[Number(value)] ?? null)
                    }}
                  >
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {selectedTemplateModels.map((model, index) => (
                        <SelectItem
                          key={`${model.id}-${model.dimensions}-${index}`}
                          value={String(index)}
                        >
                          {model.name} · {model.dimensions}d
                        </SelectItem>
                      ))}
                      <SelectItem value={CUSTOM_MODEL_VALUE}>{t("common.custom")}</SelectItem>
                    </SelectContent>
                  </Select>
                </div>
              )}
              <div
                className={
                  showCustomModelInput
                    ? "grid gap-4 sm:grid-cols-[1fr_140px]"
                    : "grid gap-1.5 sm:max-w-[140px]"
                }
              >
                {showCustomModelInput && (
                  <div className="grid gap-1.5">
                    <Label>{t("settings.memoryModel")}</Label>
                    <Input
                      value={editing.apiModel ?? ""}
                      onChange={(e) =>
                        setEditing({
                          ...editing,
                          apiModel: e.target.value,
                          source: editing.source === "ollama" ? "ollama" : "custom",
                        })
                      }
                      placeholder="text-embedding-3-small"
                    />
                  </div>
                )}
                <div className="grid gap-1.5">
                  <Label>{t("settings.memoryDimensions")}</Label>
                  <Input
                    type="number"
                    value={editing.apiDimensions ?? ""}
                    onChange={(e) =>
                      setEditing({
                        ...editing,
                        apiDimensions: e.target.value ? Number(e.target.value) : null,
                        source: editing.source === "ollama" ? "ollama" : "custom",
                      })
                    }
                    placeholder="1536"
                  />
                </div>
              </div>
            </div>
          )}
          <DialogFooter>
            {editing && (
              <Button variant="outline" onClick={() => void testModel(editing)}>
                <Wifi className="mr-1.5 h-3.5 w-3.5" />
                {t("common.test")}
              </Button>
            )}
            <Button onClick={() => void saveEditing()} disabled={saving}>
              {saving && <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />}
              {t("common.save")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <AlertDialog open={!!pendingDefault} onOpenChange={(open) => !open && setPendingDefault(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("settings.embeddingModels.confirmSwitchTitle")}</AlertDialogTitle>
            <AlertDialogDescription>
              {t("settings.embeddingModels.confirmSwitchDesc", {
                model: pendingDefault?.name ?? "",
              })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction onClick={() => void confirmDefault()}>
              {t("settings.embeddingModels.confirmSwitchAction")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      <AlertDialog open={!!pendingDelete} onOpenChange={(open) => !open && setPendingDelete(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("settings.embeddingModels.deleteTitle")}</AlertDialogTitle>
            <AlertDialogDescription>
              {t("settings.embeddingModels.deleteDesc", { model: pendingDelete?.name ?? "" })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction onClick={() => void confirmDelete()}>
              {t("common.delete")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
