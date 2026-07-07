import { Check, FileArchive, Loader2, X } from "lucide-react"
import type { TFunction } from "i18next"
import { useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"

import { ProposalDiff } from "@/components/knowledge/KnowledgeCompilePanel"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { logger } from "@/lib/logger"
import { getTransport } from "@/lib/transport-provider"
import type { Message } from "@/types/chat"
import type { CompileProposal, QueryFileMode } from "@/types/knowledge"

interface Props {
  kbId: string | null
  sessionId: string | null
  currentNotePath: string | null
  message: Message | null
  open: boolean
  onOpenChange: (open: boolean) => void
  onAfterApply?: () => void
}

const MODE_OPTIONS: Array<{ value: QueryFileMode; labelKey: string; fallback: string }> = [
  { value: "create_note", labelKey: "knowledge.queryFile.mode.createNote", fallback: "New note" },
  {
    value: "update_current_note",
    labelKey: "knowledge.queryFile.mode.updateCurrentNote",
    fallback: "Update current note",
  },
  { value: "append_to_moc", labelKey: "knowledge.queryFile.mode.appendToMoc", fallback: "Add to MOC" },
  {
    value: "append_open_questions",
    labelKey: "knowledge.queryFile.mode.openQuestions",
    fallback: "Open Questions",
  },
]

export default function KnowledgeQueryFilingDialog({
  kbId,
  sessionId,
  currentNotePath,
  message,
  open,
  onOpenChange,
  onAfterApply,
}: Props) {
  const { t } = useTranslation()
  const [mode, setMode] = useState<QueryFileMode>("create_note")
  const [title, setTitle] = useState("")
  const [targetPath, setTargetPath] = useState("")
  const [proposal, setProposal] = useState<CompileProposal | null>(null)
  const [generating, setGenerating] = useState(false)
  const [deciding, setDeciding] = useState<"apply" | "reject" | null>(null)

  const messageId = message?.dbId ?? null
  const targetRequired = mode !== "create_note"
  const canGenerate =
    !!kbId &&
    !!sessionId &&
    !!messageId &&
    title.trim().length > 0 &&
    (!targetRequired || targetPath.trim().length > 0)

  useEffect(() => {
    if (!open || !message) return
    const nextTitle = titleFromMessage(
      message.content,
      t("knowledge.queryFile.defaultTitle", "Filed conversation"),
    )
    setMode("create_note")
    setTitle(nextTitle)
    setTargetPath(defaultTargetPath("create_note", nextTitle, currentNotePath, message.dbId ?? null))
    setProposal(null)
  }, [currentNotePath, message, open, t])

  function changeMode(next: QueryFileMode) {
    setMode(next)
    setProposal(null)
    setTargetPath(defaultTargetPath(next, title, currentNotePath, messageId))
  }

  async function generateProposal() {
    if (!kbId || !sessionId || !messageId) return
    setGenerating(true)
    try {
      const result = await getTransport().call<CompileProposal>("kb_query_file_cmd", {
        kbId,
        input: {
          sessionId,
          messageId,
          mode,
          currentNotePath,
          targetPath: targetPath.trim() || null,
          title: title.trim() || null,
          confirmConversationSource: false,
        },
      })
      setProposal(result)
    } catch (e) {
      logger.warn("knowledge", "KnowledgeQueryFilingDialog::generate", "filing failed", e)
      toast.error(t("knowledge.queryFile.generateFailed", "Couldn't create filing proposal"))
    } finally {
      setGenerating(false)
    }
  }

  async function decide(approve: boolean) {
    if (!proposal || !kbId) return
    setDeciding(approve ? "apply" : "reject")
    try {
      await getTransport().call(
        approve ? "kb_compile_proposal_approve_cmd" : "kb_compile_proposal_reject_cmd",
        { kbId, id: proposal.id },
      )
      toast.success(
        approve
          ? t("knowledge.queryFile.applied", "Filed to knowledge space")
          : t("knowledge.queryFile.rejected", "Filing discarded"),
      )
      if (approve) onAfterApply?.()
      onOpenChange(false)
    } catch (e) {
      logger.warn("knowledge", "KnowledgeQueryFilingDialog::decide", "decision failed", e)
      toast.error(
        approve
          ? t("knowledge.queryFile.applyFailed", "Couldn't apply filing")
          : t("knowledge.queryFile.rejectFailed", "Couldn't discard filing"),
      )
    } finally {
      setDeciding(null)
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="flex h-[78vh] max-w-5xl flex-col gap-0 overflow-hidden p-0">
        <DialogHeader className="border-b border-border-soft/60 px-4 py-3 pr-12">
          <DialogTitle className="flex items-center gap-2 text-base">
            <FileArchive className="h-4 w-4 text-primary" />
            {t("knowledge.queryFile.title", "File answer")}
          </DialogTitle>
          <DialogDescription>
            {proposal
              ? proposalPath(proposal)
              : t("knowledge.queryFile.description", "Create a review diff before writing.")}
          </DialogDescription>
        </DialogHeader>

        <div className="grid gap-3 border-b border-border-soft/60 px-4 py-3 md:grid-cols-[180px_minmax(0,1fr)_minmax(0,1.2fr)_auto]">
          <Select value={mode} onValueChange={(value) => changeMode(value as QueryFileMode)}>
            <SelectTrigger className="h-9">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {MODE_OPTIONS.map((item) => (
                <SelectItem key={item.value} value={item.value}>
                  {t(item.labelKey, item.fallback)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <Input
            value={title}
            onChange={(e) => {
              setTitle(e.target.value)
              setProposal(null)
            }}
            placeholder={t("knowledge.queryFile.titlePlaceholder", "Title")}
          />
          <Input
            value={targetPath}
            onChange={(e) => {
              setTargetPath(e.target.value)
              setProposal(null)
            }}
            placeholder={targetPlaceholder(mode, currentNotePath, t)}
          />
          <Button
            type="button"
            className="h-9 gap-1.5"
            disabled={!canGenerate || generating}
            onClick={() => void generateProposal()}
          >
            {generating ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <FileArchive className="h-3.5 w-3.5" />
            )}
            {t("knowledge.queryFile.review", "Review")}
          </Button>
        </div>

        <div className="min-h-0 flex-1 overflow-auto bg-muted/10">
          {proposal ? (
            <ProposalDiff proposal={proposal} />
          ) : (
            <div className="flex h-full items-center justify-center px-6 text-center text-sm text-muted-foreground">
              {t("knowledge.queryFile.empty", "Choose where this answer should land.")}
            </div>
          )}
        </div>

        <DialogFooter className="border-t border-border-soft/60 px-4 py-3">
          <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
            {t("common.cancel", "Cancel")}
          </Button>
          <Button
            type="button"
            variant="outline"
            className="gap-1.5"
            disabled={!proposal || deciding != null || proposal.status !== "draft"}
            onClick={() => void decide(false)}
          >
            {deciding === "reject" ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <X className="h-3.5 w-3.5" />
            )}
            {t("knowledge.queryFile.reject", "Reject")}
          </Button>
          <Button
            type="button"
            className="gap-1.5"
            disabled={!proposal || deciding != null || proposal.status !== "draft"}
            onClick={() => void decide(true)}
          >
            {deciding === "apply" ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <Check className="h-3.5 w-3.5" />
            )}
            {t("knowledge.queryFile.apply", "Apply")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

function titleFromMessage(content: string, fallback: string): string {
  const first = content
    .split("\n")
    .map((line) => line.replace(/^#+\s*/, "").trim())
    .find(Boolean)
  return truncate(first || fallback, 80)
}

function defaultTargetPath(
  mode: QueryFileMode,
  title: string,
  currentNotePath: string | null,
  messageId: number | null,
): string {
  switch (mode) {
    case "update_current_note":
    case "append_open_questions":
      return currentNotePath ?? ""
    case "append_to_moc":
      return "MOCs/Conversation Filings.md"
    case "create_note":
    default:
      return `Filed Conversations/${slug(title)}-${messageId ?? "draft"}.md`
  }
}

function targetPlaceholder(mode: QueryFileMode, currentNotePath: string | null, t: TFunction): string {
  switch (mode) {
    case "update_current_note":
      return currentNotePath || t("knowledge.queryFile.currentNotePlaceholder", "Current note path")
    case "append_open_questions":
      return currentNotePath || t("knowledge.queryFile.openQuestionsPlaceholder", "Note with Open Questions")
    case "append_to_moc":
      return t("knowledge.queryFile.mocPlaceholder", "MOCs/Conversation Filings.md")
    case "create_note":
    default:
      return t("knowledge.queryFile.createNotePlaceholder", "Filed Conversations/example.md")
  }
}

function proposalPath(proposal: CompileProposal): string {
  const action = proposal.action
  switch (action.op) {
    case "append_link":
      return action.from_path
    case "create_moc":
    case "create_note":
    case "patch_note":
    case "set_frontmatter":
      return action.path
    default:
      return proposal.detail
  }
}

function truncate(value: string, max: number): string {
  return value.length > max ? value.slice(0, max).trimEnd() : value
}

function slug(value: string): string {
  const cleaned = value
    .toLowerCase()
    .replace(/[^a-z0-9\u4e00-\u9fa5]+/g, "-")
    .replace(/^-+|-+$/g, "")
  return cleaned || "filed-conversation"
}
