import { useRef, type KeyboardEvent } from "react"
import { useTranslation } from "react-i18next"
import { Label } from "@/components/ui/label"
import { Input } from "@/components/ui/input"
import { Button } from "@/components/ui/button"
import { X } from "lucide-react"

export default function AllowlistTagInput({
  tags,
  onTagsChange,
  inputValue,
  onInputChange,
}: {
  tags: string[]
  onTagsChange: (tags: string[]) => void
  inputValue: string
  onInputChange: (value: string) => void
}) {
  const { t } = useTranslation()
  const inputRef = useRef<HTMLInputElement>(null)

  const addTag = (raw: string) => {
    const value = raw.trim()
    if (value && !tags.includes(value)) {
      onTagsChange([...tags, value])
    }
    onInputChange("")
  }

  const removeTag = (index: number) => {
    onTagsChange(tags.filter((_, i) => i !== index))
  }

  const handleKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter" || e.key === ",") {
      e.preventDefault()
      addTag(inputValue)
    } else if (e.key === "Backspace" && !inputValue && tags.length > 0) {
      removeTag(tags.length - 1)
    }
  }

  const handlePaste = (e: React.ClipboardEvent<HTMLInputElement>) => {
    const text = e.clipboardData.getData("text")
    if (text.includes(",") || text.includes("\n")) {
      e.preventDefault()
      const newTags = text
        .split(/[\n,]/)
        .map((s) => s.trim())
        .filter(Boolean)
        .filter((s) => !tags.includes(s))
      if (newTags.length > 0) {
        onTagsChange([...tags, ...newTags])
      }
    }
  }

  return (
    <div className="space-y-2">
      <Label>{t("channels.userAllowlist")}</Label>
      <div
        className="flex flex-wrap gap-1.5 rounded-md border bg-background px-3 py-2 min-h-[38px] cursor-text"
        onClick={() => inputRef.current?.focus()}
      >
        {tags.map((tag, i) => (
          <span
            key={tag}
            className="inline-flex items-center gap-0.5 rounded bg-muted px-2 py-0.5 text-sm"
          >
            {tag}
            <Button
              type="button"
              variant="ghost"
              size="icon"
              className="ml-0.5 h-4 w-4 rounded-full p-0 hover:bg-muted-foreground/20"
              onClick={(e) => {
                e.stopPropagation()
                removeTag(i)
              }}
            >
              <X className="h-3 w-3" />
            </Button>
          </span>
        ))}
        <Input
          ref={inputRef}
          className="flex-1 min-w-[120px] h-auto border-0 bg-transparent p-0 shadow-none text-sm focus-visible:border-0"
          placeholder={tags.length === 0 ? t("channels.userAllowlistPlaceholder") : ""}
          value={inputValue}
          onChange={(e) => onInputChange(e.target.value)}
          onKeyDown={handleKeyDown}
          onPaste={handlePaste}
          onBlur={() => { if (inputValue.trim()) addTag(inputValue) }}
        />
      </div>
      <p className="text-xs text-muted-foreground">
        {t("channels.userAllowlistHint")}
      </p>
    </div>
  )
}
