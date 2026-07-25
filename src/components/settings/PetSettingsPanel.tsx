import { useCallback, useEffect, useRef, useState } from "react"
import { open } from "@tauri-apps/plugin-dialog"
import { getCurrentWindow } from "@tauri-apps/api/window"
import {
  AlertCircle,
  Check,
  Download,
  FileArchive,
  Link2,
  Loader2,
  PawPrint,
  Plus,
  RefreshCw,
  ScanSearch,
  Share2,
  Trash2,
  Upload,
} from "lucide-react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
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
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { IconTip } from "@/components/ui/tooltip"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Switch } from "@/components/ui/switch"
import { Textarea } from "@/components/ui/textarea"
import { AnimatedPetSprite, PetSprite } from "@/components/pet/PetSprite"
import { usePetAssetUrl } from "@/components/pet/hooks/usePetAssetUrl"
import { petImportFailureDiagnostic } from "@/components/settings/petImportDiagnostics"
import { isTauriMode } from "@/lib/transport"
import { getTransport } from "@/lib/transport-provider"
import { cn } from "@/lib/utils"
import { logger } from "@/lib/logger"
import type {
  PetCandidatePage,
  PetConfig,
  PetImportCandidate,
  PetImportPreview,
  PetImportSource,
  PetLibrarySnapshot,
  PetSummary,
} from "@/types/pet"

type SaveStatus = "idle" | "saving" | "saved" | "failed"
type PetDialogMode = "import" | "create"

function base64Blob(value: string, mime: string): Blob {
  const binary = atob(value)
  const chunks: ArrayBuffer[] = []
  const chunkSize = 512 * 1024
  for (let offset = 0; offset < binary.length; offset += chunkSize) {
    const slice = binary.slice(offset, offset + chunkSize)
    const buffer = new ArrayBuffer(slice.length)
    const bytes = new Uint8Array(buffer)
    for (let index = 0; index < slice.length; index += 1) bytes[index] = slice.charCodeAt(index)
    chunks.push(buffer)
  }
  return new Blob(chunks, { type: mime })
}

function localImportSources(paths: string[]): PetImportSource[] {
  if (paths.length === 1) return [{ kind: "localPath", path: paths[0] }]
  const hasManifest = paths.some((path) => path.toLowerCase().endsWith(".json"))
  const allLooseFiles = paths.every((path) => /\.(json|png|webp)$/i.test(path))
  if (hasManifest && allLooseFiles) return [{ kind: "localPaths", paths }]
  // Multiple folders, archives, or standalone atlases are independent pets.
  // A manifest plus its image is the only multi-path shape treated as one unit.
  return paths.map((path) => ({ kind: "localPath", path }))
}

const MAX_DROPPED_PET_FILES = 64
const MAX_DROPPED_PET_DEPTH = 8

function fileFromEntry(entry: FileSystemFileEntry): Promise<File> {
  return new Promise((resolve, reject) => entry.file(resolve, reject))
}

async function readDirectoryEntries(entry: FileSystemDirectoryEntry): Promise<FileSystemEntry[]> {
  const reader = entry.createReader()
  const entries: FileSystemEntry[] = []
  for (;;) {
    const batch = await new Promise<FileSystemEntry[]>((resolve, reject) =>
      reader.readEntries(resolve, reject),
    )
    if (batch.length === 0) return entries
    entries.push(...batch)
    if (entries.length > MAX_DROPPED_PET_FILES) throw new Error("pet_drop_too_many_files")
  }
}

async function collectDroppedEntry(
  entry: FileSystemEntry,
  depth: number,
  files: File[],
): Promise<void> {
  if (depth > MAX_DROPPED_PET_DEPTH) throw new Error("pet_drop_too_deep")
  if (entry.isFile) {
    files.push(await fileFromEntry(entry as FileSystemFileEntry))
    if (files.length > MAX_DROPPED_PET_FILES) throw new Error("pet_drop_too_many_files")
    return
  }
  if (!entry.isDirectory) return
  for (const child of await readDirectoryEntries(entry as FileSystemDirectoryEntry)) {
    await collectDroppedEntry(child, depth + 1, files)
  }
}

function groupLooseBrowserFiles(files: File[]): File[][] {
  if (files.length === 0) return []
  const manifests = files.filter((file) => file.name.toLowerCase().endsWith(".json"))
  const allLooseFiles = files.every((file) => /\.(json|png|webp)$/i.test(file.name))
  return manifests.length === 1 && allLooseFiles ? [files] : files.map((file) => [file])
}

async function browserDropGroups(dataTransfer: DataTransfer): Promise<File[][]> {
  // Capture entries synchronously; some WebViews clear DataTransfer after the
  // drop handler returns to the event loop.
  const entries = Array.from(dataTransfer.items)
    .filter((item) => item.kind === "file")
    .map((item) => item.webkitGetAsEntry())
    .filter((entry): entry is FileSystemEntry => entry !== null)
  if (entries.length === 0) return groupLooseBrowserFiles(Array.from(dataTransfer.files))

  const directoryGroups: File[][] = []
  const looseFiles: File[] = []
  for (const entry of entries) {
    if (entry.isDirectory) {
      const files: File[] = []
      await collectDroppedEntry(entry, 0, files)
      if (files.length > 0) directoryGroups.push(files)
    } else if (entry.isFile) {
      looseFiles.push(await fileFromEntry(entry as FileSystemFileEntry))
    }
  }
  return [...directoryGroups, ...groupLooseBrowserFiles(looseFiles)]
}

function uploadedSources(files: File[], uploadIds: string[]): PetImportSource[] {
  if (uploadIds.length === 1) return [{ kind: "uploadedPath", uploadId: uploadIds[0] }]
  const manifests = files.filter((file) => file.name.toLowerCase().endsWith(".json"))
  if (manifests.length === 1) return [{ kind: "uploadedFiles", uploadIds }]
  return uploadIds.map((uploadId) => ({ kind: "uploadedPath", uploadId }))
}

function uploadIdsForSource(source: PetImportSource): string[] {
  if (source.kind === "uploadedPath") return [source.uploadId]
  if (source.kind === "uploadedFiles") return source.uploadIds
  return []
}

function LazyPetPreview({ pet }: { pet: PetSummary }) {
  const hostRef = useRef<HTMLDivElement>(null)
  const [visible, setVisible] = useState(false)
  useEffect(() => {
    const node = hostRef.current
    if (!node) return
    const observer = new IntersectionObserver(
      ([entry]) => {
        if (entry.isIntersecting) {
          setVisible(true)
          observer.disconnect()
        }
      },
      { rootMargin: "160px" },
    )
    observer.observe(node)
    return () => observer.disconnect()
  }, [])
  const asset = usePetAssetUrl(visible ? pet.assetId : null)
  return (
    <div
      ref={hostRef}
      className="flex h-[112px] items-end justify-center overflow-hidden rounded-xl bg-muted/35"
    >
      {visible ? (
        <PetSprite
          src={asset.src}
          row={0}
          frame={0}
          rowCount={pet.manifest.spriteVersionNumber === 2 ? 11 : 9}
          dimmed={asset.loading || asset.failed}
        />
      ) : (
        <PawPrint className="mb-8 h-8 w-8 text-muted-foreground/40" aria-hidden="true" />
      )}
    </div>
  )
}

function CandidateThumbnail({ candidate }: { candidate: PetImportCandidate }) {
  const hostRef = useRef<HTMLDivElement>(null)
  const [src, setSrc] = useState<string | null>(null)
  useEffect(() => {
    const node = hostRef.current
    if (!node) return
    let objectUrl: string | null = null
    let cancelled = false
    const observer = new IntersectionObserver(
      ([entry]) => {
        if (!entry.isIntersecting) return
        observer.disconnect()
        void getTransport()
          .call<number[]>("pet_candidate_thumbnail_cmd", { candidateId: candidate.candidateId })
          .then((bytes) => {
            if (cancelled) return
            objectUrl = URL.createObjectURL(
              new Blob([new Uint8Array(bytes)], { type: "image/png" }),
            )
            setSrc(objectUrl)
          })
          .catch(() => undefined)
      },
      { rootMargin: "120px" },
    )
    observer.observe(node)
    return () => {
      cancelled = true
      observer.disconnect()
      if (objectUrl) URL.revokeObjectURL(objectUrl)
    }
  }, [candidate.candidateId])
  return (
    <div
      ref={hostRef}
      className="flex h-16 w-16 shrink-0 items-center justify-center overflow-hidden rounded-xl bg-muted/40"
    >
      {src ? (
        <img src={src} alt="" className="h-full w-full object-contain" />
      ) : (
        <PawPrint className="h-5 w-5 text-muted-foreground/40" aria-hidden="true" />
      )}
    </div>
  )
}

function ImportAnimationPreview({ previewToken }: { previewToken: string }) {
  const [src, setSrc] = useState<string | null>(null)
  useEffect(() => {
    let objectUrl: string | null = null
    let cancelled = false
    void getTransport()
      .call<number[]>("pet_preview_thumbnail_cmd", { previewToken })
      .then((bytes) => {
        if (cancelled) return
        objectUrl = URL.createObjectURL(new Blob([new Uint8Array(bytes)], { type: "image/png" }))
        setSrc(objectUrl)
      })
      .catch(() => undefined)
    return () => {
      cancelled = true
      if (objectUrl) URL.revokeObjectURL(objectUrl)
    }
  }, [previewToken])
  return (
    <div className="flex h-28 w-28 shrink-0 items-end justify-center overflow-hidden rounded-xl bg-muted/35">
      {src ? (
        <AnimatedPetSprite src={src} action="idle" rowCount={1} />
      ) : (
        <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" aria-hidden="true" />
      )}
    </div>
  )
}

export default function PetSettingsPanel({
  initialImportLink,
  onInitialImportLinkConsumed,
}: {
  initialImportLink?: string | null
  onInitialImportLinkConsumed?: () => void
}) {
  const { t } = useTranslation()
  const [config, setConfig] = useState<PetConfig | null>(null)
  const [library, setLibrary] = useState<PetLibrarySnapshot | null>(null)
  const [loading, setLoading] = useState(true)
  const [saveStatus, setSaveStatus] = useState<SaveStatus>("idle")
  const [importOpen, setImportOpen] = useState(Boolean(initialImportLink))
  const [dialogMode, setDialogMode] = useState<PetDialogMode>("import")
  const [dropActive, setDropActive] = useState(false)
  const [candidates, setCandidates] = useState<PetCandidatePage | null>(null)
  const [scanning, setScanning] = useState(false)
  const [previews, setPreviews] = useState<PetImportPreview[]>([])
  const [previewing, setPreviewing] = useState(false)
  const [committing, setCommitting] = useState(false)
  const [enableAfterImport, setEnableAfterImport] = useState(isTauriMode())
  const [deleteTarget, setDeleteTarget] = useState<PetSummary | null>(null)
  const [linkValue, setLinkValue] = useState(initialImportLink ?? "")
  const [createName, setCreateName] = useState("")
  const [createDescription, setCreateDescription] = useState("")
  const [createPrompt, setCreatePrompt] = useState("")
  const [creating, setCreating] = useState(false)
  const [exportingRef, setExportingRef] = useState<string | null>(null)
  const browserFileInputRef = useRef<HTMLInputElement>(null)
  const dropZoneRef = useRef<HTMLDivElement>(null)
  const previewRequestRevision = useRef(0)
  const creatorRequestRevision = useRef(0)
  const previewsRef = useRef<PetImportPreview[]>([])
  const consumedInitialLink = useRef<string | null>(null)
  const saveInFlight = useRef(false)
  const saveResetTimer = useRef<ReturnType<typeof setTimeout> | null>(null)

  const reload = useCallback(async () => {
    try {
      const transport = getTransport()
      const [nextConfig, nextLibrary] = await Promise.all([
        transport.call<PetConfig>("get_pet_config_cmd"),
        transport.call<PetLibrarySnapshot>("pet_list_cmd"),
      ])
      setConfig(nextConfig)
      setLibrary(nextLibrary)
    } catch (error) {
      logger.warn("pet", "PetSettingsPanel::reload", "Failed to load pet settings", error)
      toast.error(t("pet.settings.loadFailed", { defaultValue: "Could not load pets" }))
    } finally {
      setLoading(false)
    }
  }, [t])

  useEffect(() => {
    void reload()
    const transport = getTransport()
    const refresh = () => void reload()
    const unlisteners = [
      transport.listen("pet:library_changed", refresh),
      transport.listen("pet:config_changed", refresh),
    ]
    return () => {
      for (const unlisten of unlisteners) unlisten()
    }
  }, [reload])

  useEffect(
    () => () => {
      if (saveResetTimer.current) clearTimeout(saveResetTimer.current)
    },
    [],
  )

  const persist = async (next: PetConfig, field: "enabled" | "selection") => {
    if (saveInFlight.current) return
    saveInFlight.current = true
    if (saveResetTimer.current) clearTimeout(saveResetTimer.current)
    const previous = config
    setConfig(next)
    setSaveStatus("saving")
    try {
      if (field === "enabled") {
        const saved = await getTransport().call<PetConfig>("pet_set_enabled_cmd", {
          enabled: next.enabled,
          source: "settings-ui",
        })
        setConfig(saved)
      } else {
        await getTransport().call("save_pet_config_cmd", { config: next })
      }
      setSaveStatus("saved")
      saveResetTimer.current = setTimeout(() => setSaveStatus("idle"), 2_000)
    } catch (error) {
      setConfig(previous)
      setSaveStatus("failed")
      saveResetTimer.current = setTimeout(() => setSaveStatus("idle"), 2_000)
      toast.error(t("pet.settings.saveFailed", { defaultValue: "Could not save pet settings" }))
      logger.warn("pet", "PetSettingsPanel::save", "Failed to save pet settings", error)
    } finally {
      saveInFlight.current = false
    }
  }

  const replacePreviews = useCallback((next: PetImportPreview[]) => {
    previewsRef.current = next
    setPreviews(next)
  }, [])

  const discardPreviews = useCallback(async (items: PetImportPreview[]) => {
    if (items.length === 0) return
    const results = await Promise.allSettled(
      items.map((item) =>
        getTransport().call<boolean>("pet_import_preview_cancel_cmd", {
          previewToken: item.previewToken,
        }),
      ),
    )
    const failureCount = results.filter((result) => result.status === "rejected").length
    if (failureCount > 0) {
      logger.warn(
        "pet",
        "PetSettingsPanel::discardPreviews",
        "Failed to release discarded pet previews",
        { failureCount },
      )
    }
  }, [])

  const clearPreviews = useCallback(() => {
    const discarded = previewsRef.current
    replacePreviews([])
    void discardPreviews(discarded)
  }, [discardPreviews, replacePreviews])

  const removePreview = useCallback(
    (preview: PetImportPreview) => {
      replacePreviews(
        previewsRef.current.filter((item) => item.previewToken !== preview.previewToken),
      )
      void discardPreviews([preview])
    },
    [discardPreviews, replacePreviews],
  )

  useEffect(
    () => () => {
      previewRequestRevision.current += 1
      creatorRequestRevision.current += 1
      const discarded = previewsRef.current
      previewsRef.current = []
      void discardPreviews(discarded)
    },
    [discardPreviews],
  )

  const previewSources = useCallback(
    async (sources: PetImportSource[]) => {
      if (sources.length === 0) return
      const revision = ++previewRequestRevision.current
      setPreviewing(true)
      clearPreviews()
      const results = await Promise.allSettled(
        sources.map((source) =>
          getTransport().call<PetImportPreview>("pet_import_preview_cmd", {
            request: { source },
          }),
        ),
      )
      const failureDiagnostics = results.flatMap((result, index) =>
        result.status === "rejected"
          ? [petImportFailureDiagnostic(sources[index], result.reason)]
          : [],
      )
      if (failureDiagnostics.length > 0) {
        logger.warn(
          "pet",
          "PetSettingsPanel::previewSources",
          "Pet import preview request failed",
          {
            failureCount: failureDiagnostics.length,
            failures: failureDiagnostics,
          },
        )
      }
      const cleanupResults = await Promise.allSettled(
        results.flatMap((result, index) =>
          result.status === "rejected"
            ? uploadIdsForSource(sources[index]).map((id) => getTransport().discardFileUpload(id))
            : [],
        ),
      )
      const cleanupFailureCount = cleanupResults.filter(
        (result) => result.status === "rejected",
      ).length
      if (cleanupFailureCount > 0) {
        logger.warn(
          "pet",
          "PetSettingsPanel::previewSources",
          "Failed to discard uploads after a Pet preview rejection",
          {
            command: "file_upload_discard",
            failureCount: cleanupFailureCount,
          },
        )
      }
      const next = results.flatMap((result) =>
        result.status === "fulfilled" ? [result.value] : [],
      )
      if (revision === previewRequestRevision.current) {
        const failures = results.flatMap((result) =>
          result.status === "rejected" ? [String(result.reason)] : [],
        )
        replacePreviews(next)
        if (failures.length > 0) {
          toast.error(
            t("pet.import.previewFailedCount", {
              amount: failures.length,
              defaultValue: `${failures.length} pet package(s) could not be previewed`,
            }),
            { description: failures[0] },
          )
        }
        setPreviewing(false)
      } else {
        await discardPreviews(next)
      }
    },
    [clearPreviews, discardPreviews, replacePreviews, t],
  )

  const previewSource = useCallback(
    (source: PetImportSource) => previewSources([source]),
    [previewSources],
  )

  useEffect(() => {
    if (!initialImportLink) {
      consumedInitialLink.current = null
      return
    }
    if (consumedInitialLink.current === initialImportLink) return
    consumedInitialLink.current = initialImportLink
    setDialogMode("import")
    setImportOpen(true)
    setLinkValue(initialImportLink)
    onInitialImportLinkConsumed?.()
    void previewSource({ kind: "link", link: initialImportLink })
  }, [initialImportLink, onInitialImportLinkConsumed, previewSource])

  useEffect(() => {
    if (!importOpen || dialogMode !== "import" || !isTauriMode()) return
    let dispose: (() => void) | undefined
    let cancelled = false
    const currentWindow = getCurrentWindow()
    let scaleFactor = window.devicePixelRatio || 1
    void currentWindow
      .scaleFactor()
      .then((value) => {
        scaleFactor = value
      })
      .catch(() => undefined)
    const isInsideDropZone = (position: { x: number; y: number }) => {
      const rect = dropZoneRef.current?.getBoundingClientRect()
      if (!rect) return false
      const x = position.x / scaleFactor
      const y = position.y / scaleFactor
      return x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom
    }
    void currentWindow
      .onDragDropEvent((event) => {
        if (event.payload.type === "enter" || event.payload.type === "over") {
          setDropActive(isInsideDropZone(event.payload.position))
        }
        if (event.payload.type === "leave") setDropActive(false)
        if (event.payload.type === "drop") {
          setDropActive(false)
          const paths = event.payload.paths
          if (paths.length > 0 && isInsideDropZone(event.payload.position)) {
            void previewSources(localImportSources(paths))
          }
        }
      })
      .then((unlisten) => {
        if (cancelled) unlisten()
        else dispose = unlisten
      })
      .catch((error) => {
        logger.warn("pet", "native_drop_listener", "Failed to listen for pet file drops", error)
      })
    return () => {
      cancelled = true
      dispose?.()
    }
  }, [dialogMode, importOpen, previewSources])

  const scanCodex = async () => {
    setScanning(true)
    try {
      setCandidates(await getTransport().call<PetCandidatePage>("pet_codex_candidates_cmd"))
    } catch (error) {
      toast.error(t("pet.import.scanFailed", { defaultValue: "Could not scan Codex pets" }), {
        description: String(error),
      })
    } finally {
      setScanning(false)
    }
  }

  const chooseFiles = async () => {
    if (!isTauriMode()) {
      browserFileInputRef.current?.click()
      return
    }
    const selected = await open({
      multiple: true,
      directory: false,
      filters: [
        {
          name: t("pet.import.packageFiles", { defaultValue: "Pet packages" }),
          extensions: ["zip", "json", "png", "webp"],
        },
      ],
    })
    if (!selected) return
    const paths = Array.isArray(selected) ? selected : [selected]
    await previewSources(localImportSources(paths))
  }

  const generatePreview = async () => {
    if (!createName.trim() || !createPrompt.trim()) return
    const revision = ++creatorRequestRevision.current
    setCreating(true)
    clearPreviews()
    try {
      const next = await getTransport().call<PetImportPreview>("pet_create_preview_cmd", {
        request: {
          displayName: createName,
          description: createDescription.trim() || null,
          prompt: createPrompt,
        },
      })
      if (revision === creatorRequestRevision.current) replacePreviews([next])
      else await discardPreviews([next])
    } catch (error) {
      if (revision === creatorRequestRevision.current) {
        toast.error(t("pet.creator.failed", { defaultValue: "Could not create this pet" }), {
          description: String(error),
        })
      }
    } finally {
      if (revision === creatorRequestRevision.current) setCreating(false)
    }
  }

  const exportPet = async (pet: PetSummary) => {
    setExportingRef(pet.petRef)
    try {
      const result = await getTransport().call<{
        fileName: string
        mime: string
        dataBase64: string
      }>("pet_export_cmd", { petRef: pet.petRef })
      const saved = await getTransport().saveFileAs(
        base64Blob(result.dataBase64, result.mime),
        result.fileName,
      )
      if (saved.status !== "canceled") {
        toast.success(t("pet.export.saved", { defaultValue: "Codex-compatible package exported" }))
      }
    } catch (error) {
      toast.error(t("pet.export.failed", { defaultValue: "Could not export this pet" }), {
        description: String(error),
      })
    } finally {
      setExportingRef(null)
    }
  }

  const openDialog = (mode: PetDialogMode) => {
    previewRequestRevision.current += 1
    creatorRequestRevision.current += 1
    setDialogMode(mode)
    clearPreviews()
    setDropActive(false)
    setImportOpen(true)
  }

  const closeDialog = () => {
    previewRequestRevision.current += 1
    creatorRequestRevision.current += 1
    setImportOpen(false)
    setPreviewing(false)
    setCreating(false)
    clearPreviews()
    setDropActive(false)
  }

  const uploadBrowserGroups = async (groups: File[][]) => {
    if (groups.length === 0) return
    setPreviewing(true)
    const uploadIds: string[] = []
    try {
      const sources: PetImportSource[] = []
      for (const files of groups) {
        const groupIds: string[] = []
        for (const file of files) {
          const lease = await getTransport().uploadFile(file, "pet_package")
          uploadIds.push(lease.uploadId)
          groupIds.push(lease.uploadId)
        }
        sources.push(...uploadedSources(files, groupIds))
      }
      await previewSources(sources)
    } catch (error) {
      await Promise.allSettled(uploadIds.map((id) => getTransport().discardFileUpload(id)))
      toast.error(t("pet.import.uploadFailed", { defaultValue: "Could not upload pet package" }), {
        description: String(error),
      })
    } finally {
      setPreviewing(false)
    }
  }

  const uploadBrowserFiles = (files: File[]) => uploadBrowserGroups([files])

  const commitPreview = async () => {
    if (previews.length === 0) return
    setCommitting(true)
    const failed: PetImportPreview[] = []
    let imported = 0
    let duplicate = 0
    let firstError: unknown = null
    for (const [index, item] of previews.entries()) {
      try {
        const result = await getTransport().call<{ imported: boolean }>("pet_import_commit_cmd", {
          request: {
            previewToken: item.previewToken,
            enableAfterImport: enableAfterImport && index === previews.length - 1,
          },
        })
        if (result.imported) imported += 1
        else duplicate += 1
      } catch (error) {
        failed.push(item)
        firstError ??= error
      }
    }
    if (imported + duplicate > 0) {
      toast.success(
        previews.length === 1
          ? imported > 0
            ? t("pet.import.imported", { defaultValue: "Pet imported" })
            : t("pet.import.alreadyImported", {
                defaultValue: "This pet is already in your library",
              })
          : t("pet.import.importedCount", {
              amount: imported + duplicate,
              defaultValue: `${imported + duplicate} pets added to your library`,
            }),
      )
      await reload()
    }
    if (failed.length > 0) {
      replacePreviews(failed)
      toast.error(
        t("pet.import.commitFailedCount", {
          amount: failed.length,
          defaultValue: `${failed.length} pet(s) could not be imported`,
        }),
        { description: String(firstError) },
      )
    } else {
      closeDialog()
    }
    setCommitting(false)
  }

  const confirmDelete = async () => {
    if (!deleteTarget) return
    const target = deleteTarget
    setDeleteTarget(null)
    try {
      const result = await getTransport().call<{ restoreToken: string }>("pet_delete_cmd", {
        petRef: target.petRef,
        expectedPackageHash: target.packageHash,
      })
      await reload()
      toast.success(t("pet.settings.deleted", { defaultValue: "Pet removed" }), {
        action: {
          label: t("pet.settings.undo", { defaultValue: "Undo" }),
          onClick: () => {
            void getTransport()
              .call("pet_restore_cmd", { request: { restoreToken: result.restoreToken } })
              .then(() => reload())
              .catch((error) => {
                toast.error(
                  t("pet.settings.saveFailed", { defaultValue: "Could not save pet settings" }),
                  { description: String(error) },
                )
                logger.warn(
                  "pet",
                  "PetSettingsPanel::restore",
                  "Failed to restore deleted pet",
                  error,
                )
              })
          },
        },
      })
    } catch (error) {
      toast.error(t("pet.settings.deleteFailed", { defaultValue: "Could not remove pet" }), {
        description: String(error),
      })
    }
  }

  if (loading || !config || !library) {
    return (
      <div className="flex flex-1 items-center justify-center">
        <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" aria-hidden="true" />
      </div>
    )
  }

  const previewHasErrors = previews.some((preview) =>
    preview.issues.some((issue) => issue.severity === "error"),
  )

  return (
    <div className="flex-1 overflow-y-auto px-6 pb-8 pt-4">
      <div className="mx-auto max-w-4xl space-y-6">
        <section className="flex items-center justify-between gap-4 rounded-xl border border-border/70 bg-card p-4">
          <div className="min-w-0">
            <h2 className="text-sm font-semibold">
              {t("pet.settings.wake", { defaultValue: "Desktop pet" })}
            </h2>
            <p className="mt-1 text-xs text-muted-foreground">
              {t("pet.settings.wakeDesc", {
                defaultValue:
                  "Show your animated companion and conversation activity on the desktop.",
              })}
            </p>
            {!isTauriMode() && (
              <p className="mt-1 text-xs text-muted-foreground">
                {t("pet.settings.desktopOnly", {
                  defaultValue: "Wake and Tuck Away are available in the desktop app.",
                })}
              </p>
            )}
          </div>
          <div className="flex shrink-0 items-center gap-2">
            {saveStatus === "saving" && (
              <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" aria-hidden="true" />
            )}
            {saveStatus === "saved" && (
              <Check className="h-4 w-4 text-emerald-500" aria-hidden="true" />
            )}
            {saveStatus === "failed" && (
              <AlertCircle className="h-4 w-4 text-destructive" aria-hidden="true" />
            )}
            <Switch
              checked={config.enabled}
              disabled={saveStatus === "saving" || !isTauriMode()}
              onCheckedChange={(enabled) => void persist({ ...config, enabled }, "enabled")}
              aria-label={t("pet.settings.wake", { defaultValue: "Desktop pet" })}
            />
          </div>
        </section>

        <section className="space-y-3">
          <div className="flex items-center justify-between gap-3">
            <div>
              <h2 className="text-sm font-semibold">
                {t("pet.settings.choose", { defaultValue: "Choose a pet" })}
              </h2>
              <p className="mt-0.5 text-xs text-muted-foreground">
                {t("pet.settings.localOnly", { defaultValue: "Custom pets stay on this device." })}
              </p>
            </div>
            <div className="flex items-center gap-1">
              <IconTip label={t("common.refresh", { defaultValue: "Refresh" })}>
                <Button
                  type="button"
                  variant="ghost"
                  size="icon"
                  onClick={() => void reload()}
                  className="h-8 w-8"
                >
                  <RefreshCw className="h-4 w-4" aria-hidden="true" />
                </Button>
              </IconTip>
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={() => openDialog("create")}
                className="gap-1.5"
              >
                <Plus className="h-3.5 w-3.5" aria-hidden="true" />
                {t("pet.creator.action", { defaultValue: "Create" })}
              </Button>
              <Button
                type="button"
                size="sm"
                onClick={() => openDialog("import")}
                className="gap-1.5"
              >
                <Download className="h-3.5 w-3.5" aria-hidden="true" />
                {t("pet.import.action", { defaultValue: "Import" })}
              </Button>
            </div>
          </div>
          <div className="grid grid-cols-2 gap-3 md:grid-cols-3">
            {library.pets.map((pet) => {
              const selected = pet.petRef === config.selectedPetRef
              return (
                <article
                  key={pet.petRef}
                  className={cn(
                    "relative rounded-2xl border bg-card p-2 transition-colors",
                    "border-border/70",
                    selected && "bg-primary/5",
                  )}
                >
                  <Button
                    type="button"
                    variant="ghost"
                    disabled={saveStatus === "saving"}
                    onClick={() =>
                      void persist({ ...config, selectedPetRef: pet.petRef }, "selection")
                    }
                    className="h-auto w-full justify-start rounded-xl p-0 text-left disabled:cursor-wait disabled:opacity-70"
                    aria-pressed={selected}
                  >
                    <LazyPetPreview pet={pet} />
                    <span className="mt-2 flex items-center justify-between gap-2 px-1 pb-1">
                      <span className="min-w-0">
                        <span className="block truncate text-sm font-medium">
                          {pet.manifest.displayName}
                        </span>
                        <span className="block text-xs text-muted-foreground">
                          v{pet.manifest.spriteVersionNumber}
                        </span>
                      </span>
                      {selected && (
                        <Check className="h-4 w-4 shrink-0 text-primary" aria-hidden="true" />
                      )}
                    </span>
                  </Button>
                  <div className="absolute right-3 top-3 flex items-center gap-1">
                    <IconTip label={t("pet.export.action", { defaultValue: "Export for Codex" })}>
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon"
                        disabled={exportingRef !== null}
                        onClick={() => void exportPet(pet)}
                        className="h-7 w-7 bg-background/85 text-muted-foreground backdrop-blur-sm"
                      >
                        {exportingRef === pet.petRef ? (
                          <Loader2 className="h-3.5 w-3.5 animate-spin" aria-hidden="true" />
                        ) : (
                          <Share2 className="h-3.5 w-3.5" aria-hidden="true" />
                        )}
                      </Button>
                    </IconTip>
                    {!pet.builtin && (
                      <IconTip
                        label={
                          selected
                            ? t("pet.settings.cannotDeleteSelected", {
                                defaultValue: "Choose another pet before removing this one",
                              })
                            : t("common.delete", { defaultValue: "Delete" })
                        }
                      >
                        <Button
                          type="button"
                          variant="ghost"
                          size="icon"
                          disabled={selected}
                          onClick={() => setDeleteTarget(pet)}
                          className="h-7 w-7 bg-background/85 text-muted-foreground backdrop-blur-sm hover:text-destructive"
                        >
                          <Trash2 className="h-3.5 w-3.5" aria-hidden="true" />
                        </Button>
                      </IconTip>
                    )}
                  </div>
                </article>
              )
            })}
          </div>
        </section>
      </div>

      <Dialog
        open={importOpen}
        onOpenChange={(openState) => {
          if (openState) setImportOpen(true)
          else if (!committing) closeDialog()
        }}
      >
        <DialogContent className="max-h-[86vh] max-w-2xl overflow-y-auto">
          <DialogHeader>
            <DialogTitle>
              {dialogMode === "create"
                ? t("pet.creator.title", { defaultValue: "Create a pet" })
                : t("pet.import.title", { defaultValue: "Import a pet" })}
            </DialogTitle>
            <DialogDescription>
              {dialogMode === "create"
                ? t("pet.creator.description", {
                    defaultValue:
                      "Generate an original mascot, then preview the Codex-compatible animation before installing it.",
                  })
                : t("pet.import.description", {
                    defaultValue:
                      "Preview and validate Codex-compatible pets before adding them to Hope.",
                  })}
            </DialogDescription>
          </DialogHeader>

          {dialogMode === "import" ? (
            <>
              <div
                ref={dropZoneRef}
                onDragEnter={(event) => {
                  event.preventDefault()
                  setDropActive(true)
                }}
                onDragOver={(event) => event.preventDefault()}
                onDragLeave={() => setDropActive(false)}
                onDrop={(event) => {
                  event.preventDefault()
                  setDropActive(false)
                  void browserDropGroups(event.dataTransfer)
                    .then(uploadBrowserGroups)
                    .catch((error) => {
                      toast.error(
                        t("pet.import.uploadFailed", {
                          defaultValue: "Could not upload pet package",
                        }),
                        { description: String(error) },
                      )
                    })
                }}
                className={cn(
                  "flex min-h-28 flex-col items-center justify-center rounded-2xl border border-dashed p-5 text-center transition-colors",
                  dropActive ? "border-primary bg-primary/10" : "border-border bg-muted/25",
                )}
              >
                <Upload className="h-6 w-6 text-muted-foreground" aria-hidden="true" />
                <p className="mt-2 text-sm font-medium">
                  {t("pet.import.drop", {
                    defaultValue: "Drop a pet folder, zip, manifest + image, PNG, or WebP",
                  })}
                </p>
                <p className="mt-1 text-xs text-muted-foreground">
                  {t("pet.import.dropSafety", {
                    defaultValue:
                      "Dropping only creates a preview. Nothing is installed until you confirm.",
                  })}
                </p>
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  onClick={() => void chooseFiles()}
                  className="mt-3 gap-1.5"
                >
                  <FileArchive className="h-3.5 w-3.5" aria-hidden="true" />
                  {t("pet.import.chooseFiles", { defaultValue: "Choose files" })}
                </Button>
                <Input
                  ref={browserFileInputRef}
                  type="file"
                  multiple
                  accept=".zip,.json,.png,.webp"
                  className="hidden"
                  onChange={(event) => {
                    const files = Array.from(event.currentTarget.files ?? [])
                    event.currentTarget.value = ""
                    void uploadBrowserFiles(files)
                  }}
                />
              </div>

              <div className="flex items-center gap-2">
                <div className="relative min-w-0 flex-1">
                  <Link2
                    className="pointer-events-none absolute left-3 top-2.5 h-4 w-4 text-muted-foreground"
                    aria-hidden="true"
                  />
                  <Input
                    value={linkValue}
                    onChange={(event) => setLinkValue(event.target.value)}
                    placeholder={t("pet.import.linkPlaceholder", {
                      defaultValue: "Paste a codex:// install link or HTTPS spritesheet URL",
                    })}
                    aria-label={t("pet.import.linkLabel", { defaultValue: "Pet link" })}
                    className="pl-9"
                  />
                </div>
                <Button
                  type="button"
                  variant="outline"
                  disabled={!linkValue.trim() || previewing}
                  onClick={() => void previewSource({ kind: "link", link: linkValue })}
                >
                  {t("pet.import.previewLink", { defaultValue: "Preview link" })}
                </Button>
              </div>

              <div className="flex items-center gap-2">
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  onClick={() => void scanCodex()}
                  disabled={scanning}
                  className="gap-1.5"
                >
                  {scanning ? (
                    <Loader2 className="h-3.5 w-3.5 animate-spin" aria-hidden="true" />
                  ) : (
                    <ScanSearch className="h-3.5 w-3.5" aria-hidden="true" />
                  )}
                  {t("pet.import.fromCodex", { defaultValue: "Scan Codex pets" })}
                </Button>
                {candidates && (
                  <span className="text-xs text-muted-foreground">
                    {t("pet.import.found", {
                      count: candidates.total,
                      defaultValue: `${candidates.total} found`,
                    })}
                  </span>
                )}
              </div>

              {candidates && candidates.candidates.length > 0 && previews.length === 0 && (
                <ul className="max-h-64 space-y-2 overflow-y-auto rounded-xl border border-border/70 p-2">
                  {candidates.candidates.map((candidate) => (
                    <li key={candidate.candidateId}>
                      <Button
                        type="button"
                        variant="ghost"
                        onClick={() =>
                          void previewSource({
                            kind: "candidate",
                            candidateId: candidate.candidateId,
                          })
                        }
                        className="h-auto w-full justify-start gap-3 rounded-xl p-2 text-left hover:bg-accent"
                      >
                        <CandidateThumbnail candidate={candidate} />
                        <span className="min-w-0 flex-1">
                          <span className="block truncate text-sm font-medium">
                            {candidate.displayName}
                          </span>
                          <span className="block text-xs text-muted-foreground">
                            {candidate.width} × {candidate.height} ·{" "}
                            {candidate.inferredVersion
                              ? `v${candidate.inferredVersion}`
                              : t("pet.import.unknownVersion", { defaultValue: "unknown version" })}
                          </span>
                        </span>
                        {candidate.issues.some((issue) => issue.severity === "error") && (
                          <AlertCircle
                            className="h-4 w-4 shrink-0 text-destructive"
                            aria-hidden="true"
                          />
                        )}
                      </Button>
                    </li>
                  ))}
                </ul>
              )}
            </>
          ) : (
            <div className="space-y-4">
              <div className="space-y-1.5">
                <Label htmlFor="pet-create-name">
                  {t("pet.creator.name", { defaultValue: "Name" })}
                </Label>
                <Input
                  id="pet-create-name"
                  value={createName}
                  maxLength={256}
                  onChange={(event) => setCreateName(event.target.value)}
                />
              </div>
              <div className="space-y-1.5">
                <Label htmlFor="pet-create-description">
                  {t("pet.creator.shortDescription", {
                    defaultValue: "Short description (optional)",
                  })}
                </Label>
                <Input
                  id="pet-create-description"
                  value={createDescription}
                  maxLength={2048}
                  onChange={(event) => setCreateDescription(event.target.value)}
                />
              </div>
              <div className="space-y-1.5">
                <Label htmlFor="pet-create-prompt">
                  {t("pet.creator.prompt", { defaultValue: "What should your pet look like?" })}
                </Label>
                <Textarea
                  id="pet-create-prompt"
                  value={createPrompt}
                  maxLength={4000}
                  rows={4}
                  onChange={(event) => setCreatePrompt(event.target.value)}
                />
              </div>
              <Button
                type="button"
                variant="outline"
                disabled={!createName.trim() || !createPrompt.trim() || creating}
                onClick={() => void generatePreview()}
                className="gap-1.5"
              >
                {creating ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin" aria-hidden="true" />
                ) : (
                  <PawPrint className="h-3.5 w-3.5" aria-hidden="true" />
                )}
                {creating
                  ? t("pet.creator.generating", { defaultValue: "Creating preview…" })
                  : t("pet.creator.generate", { defaultValue: "Generate preview" })}
              </Button>
            </div>
          )}

          {previewing && (
            <div className="flex items-center justify-center py-6">
              <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" aria-hidden="true" />
            </div>
          )}

          {previews.length > 0 && (
            <section className="space-y-3">
              {previews.length > 1 && (
                <p className="text-xs font-medium text-muted-foreground">
                  {t("pet.import.previewCount", {
                    amount: previews.length,
                    defaultValue: `${previews.length} pets ready to review`,
                  })}
                </p>
              )}
              {previews.map((preview) => (
                <article
                  key={preview.previewToken}
                  className="rounded-xl border border-border/70 bg-card p-4"
                >
                  <div className="flex items-start gap-3">
                    <ImportAnimationPreview previewToken={preview.previewToken} />
                    <div className="min-w-0 flex-1">
                      <div className="flex items-start justify-between gap-3">
                        <div>
                          <h3 className="text-sm font-semibold">{preview.manifest.displayName}</h3>
                          <p className="mt-1 text-xs text-muted-foreground">
                            {preview.width} × {preview.height} · v
                            {preview.manifest.spriteVersionNumber}
                          </p>
                        </div>
                        <div className="flex items-center gap-1">
                          {preview.duplicatePetRef && (
                            <span className="rounded-full bg-muted px-2 py-1 text-xs text-muted-foreground">
                              {t("pet.import.duplicate", { defaultValue: "Already imported" })}
                            </span>
                          )}
                          {previews.length > 1 && (
                            <IconTip
                              label={t("pet.import.removePreview", {
                                defaultValue: "Remove from this import",
                              })}
                            >
                              <Button
                                type="button"
                                variant="ghost"
                                size="icon"
                                className="h-7 w-7 text-muted-foreground"
                                onClick={() => removePreview(preview)}
                              >
                                <Trash2 className="h-3.5 w-3.5" aria-hidden="true" />
                              </Button>
                            </IconTip>
                          )}
                        </div>
                      </div>
                    </div>
                  </div>
                  {preview.issues.length > 0 && (
                    <ul className="mt-3 space-y-1.5">
                      {preview.issues.map((issue) => (
                        <li
                          key={issue.code}
                          className={cn(
                            "flex items-start gap-2 text-xs",
                            issue.severity === "error"
                              ? "text-destructive"
                              : "text-amber-700 dark:text-amber-400",
                          )}
                        >
                          <AlertCircle className="mt-0.5 h-3.5 w-3.5 shrink-0" aria-hidden="true" />
                          <span>{issue.message}</span>
                        </li>
                      ))}
                    </ul>
                  )}
                </article>
              ))}
              <div className="mt-4 flex items-center justify-between gap-3 rounded-lg bg-muted/40 px-3 py-2">
                <span className="text-xs">
                  {t("pet.import.enableAfter", { defaultValue: "Use this pet after import" })}
                </span>
                <Switch
                  checked={enableAfterImport}
                  disabled={!isTauriMode()}
                  onCheckedChange={setEnableAfterImport}
                />
              </div>
            </section>
          )}

          <DialogFooter>
            <Button type="button" variant="ghost" disabled={committing} onClick={closeDialog}>
              {t("common.cancel", { defaultValue: "Cancel" })}
            </Button>
            <Button
              type="button"
              disabled={previews.length === 0 || previewHasErrors || committing}
              onClick={() => void commitPreview()}
              className="gap-1.5"
            >
              {committing && <Loader2 className="h-3.5 w-3.5 animate-spin" aria-hidden="true" />}
              {t("pet.import.confirm", { defaultValue: "Import pet" })}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <AlertDialog
        open={deleteTarget !== null}
        onOpenChange={(openState) => {
          if (!openState) setDeleteTarget(null)
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t("pet.settings.deleteTitle", { defaultValue: "Remove this pet?" })}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {t("pet.settings.deleteDescription", {
                name: deleteTarget?.manifest.displayName ?? "",
                defaultValue: `Remove ${deleteTarget?.manifest.displayName ?? "this pet"} from this device?`,
              })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel", { defaultValue: "Cancel" })}</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => void confirmDelete()}
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
            >
              {t("common.delete", { defaultValue: "Delete" })}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
