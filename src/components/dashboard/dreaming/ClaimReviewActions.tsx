import { useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import {
  Check,
  Clock,
  Loader2,
  Pencil,
  Pin,
  PinOff,
  Trash2,
  X,
  MoveRight,
} from "lucide-react"
import { Button } from "@/components/ui/button"
import { Textarea } from "@/components/ui/textarea"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { RadioPills } from "@/components/ui/radio-pills"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
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
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import {
  claimReviewActionErrorToast,
  type ClaimReviewActionOperation,
} from "./claimReviewActionFeedback"

/** Salience at/above which a claim is force-injected (mirrors ha-core
 * `PINNED_MIN_SALIENCE`). A claim above this reads as "pinned" in the UI. */
const PINNED_MIN_SALIENCE = 0.7

/** The claim fields the review actions need. Both the Settings claims browser
 * and the Dashboard review queue conform to this shape. */
export interface ReviewableClaim {
  id: string
  scopeType: string
  scopeId?: string | null
  content: string
  subject: string
  predicate: string
  object: string
  tags: string[]
  salience: number
  status: string
}

interface ClaimReviewActionsProps {
  claim: ReviewableClaim
  /** Called after any successful mutation so the caller can refresh. */
  onChanged: () => void
}

/**
 * The user-correction toolbar for a single claim (Lucid Review, design §5.2 /
 * §5.3): approve / mark-outdated / pin-unpin / edit / move-scope / reject /
 * forget. Each action calls the owner-plane `claim_update` / `claim_forget`
 * transport, which writes evidence + a decision-log entry server-side, so the
 * correction shows up in the run history and influences the next prompt.
 */
export default function ClaimReviewActions({ claim, onChanged }: ClaimReviewActionsProps) {
  const { t } = useTranslation()
  const [busy, setBusy] = useState(false)

  // Edit dialog state.
  const [editOpen, setEditOpen] = useState(false)
  const [editContent, setEditContent] = useState("")
  const [editTags, setEditTags] = useState("")

  // Move-scope dialog state.
  const [scopeOpen, setScopeOpen] = useState(false)
  const [scopeType, setScopeType] = useState<string>("global")
  const [scopeId, setScopeId] = useState("")

  // Forget dialog state.
  const [forgetOpen, setForgetOpen] = useState(false)
  const [forgetMode, setForgetMode] = useState<"archive" | "permanent">("archive")
  const [forgetNote, setForgetNote] = useState("")

  // Reject confirm state.
  const [rejectOpen, setRejectOpen] = useState(false)

  const isPinned = claim.salience >= PINNED_MIN_SALIENCE
  const isActive = claim.status === "active"

  const run = async (
    operation: Exclude<ClaimReviewActionOperation, "loadQueue">,
    fn: () => Promise<unknown>,
    successMsg: string,
    after?: () => void,
  ) => {
    if (busy) return
    setBusy(true)
    try {
      await fn()
      toast.success(successMsg)
      after?.()
      onChanged()
    } catch (e) {
      logger.error("dashboard", "ClaimReviewActions", "claim action failed", e)
      const failure = claimReviewActionErrorToast(operation, t, e)
      toast.error(failure.title, failure.description ? { description: failure.description } : undefined)
    } finally {
      setBusy(false)
    }
  }

  const patch = (args: Record<string, unknown>) =>
    getTransport().call("claim_update", { id: claim.id, ...args })

  const openEdit = () => {
    setEditContent(claim.content)
    setEditTags(claim.tags.join(", "))
    setEditOpen(true)
  }
  const openScope = () => {
    setScopeType(claim.scopeType)
    setScopeId(claim.scopeId ?? "")
    setScopeOpen(true)
  }
  const openForget = () => {
    setForgetMode("archive")
    setForgetNote("")
    setForgetOpen(true)
  }

  const submitEdit = () =>
    run(
      "edit",
      () =>
        patch({
          content: editContent.trim(),
          tags: editTags
            .split(",")
            .map((s) => s.trim())
            .filter(Boolean),
        }),
      t("dashboard.dreaming.review.editDone"),
      () => setEditOpen(false),
    )

  const submitScope = () =>
    run(
      "moveScope",
      () =>
        patch({
          scopeType,
          scopeId: scopeType === "global" ? undefined : scopeId.trim(),
        }),
      t("dashboard.dreaming.review.moveScopeDone"),
      () => setScopeOpen(false),
    )

  const submitForget = () =>
    run(
      "forget",
      () =>
        getTransport().call("claim_forget", {
          id: claim.id,
          permanent: forgetMode === "permanent",
          note: forgetNote.trim() || undefined,
        }),
      t("dashboard.dreaming.review.forgetDone"),
      () => setForgetOpen(false),
    )

  return (
    <div className="flex flex-wrap items-center gap-1.5">
      {!isActive && (
        <Button
          size="sm"
          variant="outline"
          className="h-7 gap-1 text-xs"
          disabled={busy}
          onClick={() =>
            run(
              "approve",
              () => patch({ status: "active" }),
              t("dashboard.dreaming.review.approveDone"),
            )
          }
        >
          <Check className="h-3 w-3 text-emerald-500" />
          {t("dashboard.dreaming.review.approve")}
        </Button>
      )}
      {isActive && (
        <Button
          size="sm"
          variant="outline"
          className="h-7 gap-1 text-xs"
          disabled={busy}
          onClick={() =>
            run(
              "markOutdated",
              () => patch({ status: "expired" }),
              t("dashboard.dreaming.review.markOutdatedDone"),
            )
          }
        >
          <Clock className="h-3 w-3" />
          {t("dashboard.dreaming.review.markOutdated")}
        </Button>
      )}
      {/* Pin only matters for active claims — `list_pinned_claims` (and the
          Context Pack static segment) is active-only, so pinning a
          needs_review / archived claim would silently no-op. */}
      {isActive && (
        <Button
          size="sm"
          variant="outline"
          className="h-7 gap-1 text-xs"
          disabled={busy}
          onClick={() =>
            run(
              isPinned ? "unpin" : "pin",
              () => patch({ pinned: !isPinned }),
              isPinned
                ? t("dashboard.dreaming.review.unpinDone")
                : t("dashboard.dreaming.review.pinDone"),
            )
          }
        >
          {isPinned ? <PinOff className="h-3 w-3" /> : <Pin className="h-3 w-3" />}
          {isPinned ? t("dashboard.dreaming.review.unpin") : t("dashboard.dreaming.review.pin")}
        </Button>
      )}
      <Button
        size="sm"
        variant="outline"
        className="h-7 gap-1 text-xs"
        disabled={busy}
        onClick={openEdit}
      >
        <Pencil className="h-3 w-3" />
        {t("dashboard.dreaming.review.edit")}
      </Button>
      <Button
        size="sm"
        variant="outline"
        className="h-7 gap-1 text-xs"
        disabled={busy}
        onClick={openScope}
      >
        <MoveRight className="h-3 w-3" />
        {t("dashboard.dreaming.review.moveScope")}
      </Button>
      {claim.status !== "archived" && (
        <Button
          size="sm"
          variant="outline"
          className="h-7 gap-1 text-xs"
          disabled={busy}
          onClick={() => setRejectOpen(true)}
        >
          <X className="h-3 w-3 text-amber-500" />
          {t("dashboard.dreaming.review.reject")}
        </Button>
      )}
      <Button
        size="sm"
        variant="outline"
        className="h-7 gap-1 text-xs text-destructive hover:text-destructive"
        disabled={busy}
        onClick={openForget}
      >
        <Trash2 className="h-3 w-3" />
        {t("dashboard.dreaming.review.forget")}
      </Button>
      {busy && <Loader2 className="h-3.5 w-3.5 animate-spin text-muted-foreground" />}

      {/* Edit dialog */}
      <Dialog open={editOpen} onOpenChange={(o) => !busy && setEditOpen(o)}>
        <DialogContent className="max-w-lg">
          <DialogHeader>
            <DialogTitle>{t("dashboard.dreaming.review.editTitle")}</DialogTitle>
            <DialogDescription>{t("dashboard.dreaming.review.editDesc")}</DialogDescription>
          </DialogHeader>
          <div className="space-y-3">
            <div className="space-y-1.5">
              <Label className="text-xs">{t("dashboard.dreaming.review.editContent")}</Label>
              <Textarea
                value={editContent}
                onChange={(e) => setEditContent(e.target.value)}
                rows={3}
                className="text-sm"
              />
            </div>
            <div className="space-y-1.5">
              <Label className="text-xs">{t("dashboard.dreaming.review.editTags")}</Label>
              <Input
                value={editTags}
                onChange={(e) => setEditTags(e.target.value)}
                placeholder={t("dashboard.dreaming.review.editTagsPlaceholder")}
                className="text-sm"
              />
            </div>
          </div>
          <DialogFooter>
            <Button variant="ghost" size="sm" onClick={() => setEditOpen(false)} disabled={busy}>
              {t("common.cancel")}
            </Button>
            <Button
              size="sm"
              onClick={submitEdit}
              disabled={busy || !editContent.trim()}
            >
              {busy ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : t("common.save")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Move-scope dialog */}
      <Dialog open={scopeOpen} onOpenChange={(o) => !busy && setScopeOpen(o)}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>{t("dashboard.dreaming.review.moveScopeTitle")}</DialogTitle>
            <DialogDescription>{t("dashboard.dreaming.review.moveScopeDesc")}</DialogDescription>
          </DialogHeader>
          <div className="space-y-3">
            <Select value={scopeType} onValueChange={setScopeType}>
              <SelectTrigger className="text-sm">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="global">{t("dashboard.dreaming.review.scopeGlobal")}</SelectItem>
                <SelectItem value="agent">{t("dashboard.dreaming.review.scopeAgent")}</SelectItem>
                <SelectItem value="project">
                  {t("dashboard.dreaming.review.scopeProject")}
                </SelectItem>
              </SelectContent>
            </Select>
            {scopeType !== "global" && (
              <div className="space-y-1.5">
                <Label className="text-xs">{t("dashboard.dreaming.review.scopeId")}</Label>
                <Input
                  value={scopeId}
                  onChange={(e) => setScopeId(e.target.value)}
                  placeholder={t("dashboard.dreaming.review.scopeIdPlaceholder")}
                  className="text-sm"
                />
              </div>
            )}
          </div>
          <DialogFooter>
            <Button variant="ghost" size="sm" onClick={() => setScopeOpen(false)} disabled={busy}>
              {t("common.cancel")}
            </Button>
            <Button
              size="sm"
              onClick={submitScope}
              disabled={busy || (scopeType !== "global" && !scopeId.trim())}
            >
              {busy ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : t("common.save")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Forget dialog */}
      <Dialog open={forgetOpen} onOpenChange={(o) => !busy && setForgetOpen(o)}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>{t("dashboard.dreaming.review.forgetTitle")}</DialogTitle>
            <DialogDescription>{t("dashboard.dreaming.review.forgetDesc")}</DialogDescription>
          </DialogHeader>
          <div className="space-y-3">
            <RadioPills
              value={forgetMode}
              onChange={(v) => setForgetMode(v)}
              options={[
                { value: "archive", label: t("dashboard.dreaming.review.forgetArchive") },
                { value: "permanent", label: t("dashboard.dreaming.review.forgetPermanent") },
              ]}
            />
            <p className="text-xs text-muted-foreground">
              {forgetMode === "permanent"
                ? t("dashboard.dreaming.review.forgetPermanentDesc")
                : t("dashboard.dreaming.review.forgetArchiveDesc")}
            </p>
            <Textarea
              value={forgetNote}
              onChange={(e) => setForgetNote(e.target.value)}
              rows={2}
              placeholder={t("dashboard.dreaming.review.forgetNotePlaceholder")}
              className="text-sm"
            />
          </div>
          <DialogFooter>
            <Button variant="ghost" size="sm" onClick={() => setForgetOpen(false)} disabled={busy}>
              {t("common.cancel")}
            </Button>
            <Button
              size="sm"
              variant={forgetMode === "permanent" ? "destructive" : "default"}
              onClick={submitForget}
              disabled={busy}
            >
              {busy ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
              ) : (
                t("dashboard.dreaming.review.forgetConfirm")
              )}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Reject confirm */}
      <AlertDialog open={rejectOpen} onOpenChange={(o) => !busy && setRejectOpen(o)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("dashboard.dreaming.review.rejectTitle")}</AlertDialogTitle>
            <AlertDialogDescription>
              {t("dashboard.dreaming.review.rejectDesc")}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={busy}>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction
              onClick={(e) => {
                e.preventDefault()
                void run(
                  "reject",
                  () => patch({ status: "archived" }),
                  t("dashboard.dreaming.review.rejectDone"),
                  () => setRejectOpen(false),
                )
              }}
            >
              {t("dashboard.dreaming.review.reject")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
