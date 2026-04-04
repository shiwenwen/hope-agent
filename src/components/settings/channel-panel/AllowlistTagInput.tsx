import { useRef, type KeyboardEvent } from "react"
import { useTranslation } from "react-i18next"
import { Label } from "@/components/ui/label"
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
            <button
              type="button"
              className="ml-0.5 rounded-full hover:bg-muted-foreground/20 p-0.5"
              onClick={(e) => {
                e.stopPropagation()
                removeTag(i)
              }}
            >
              <X className="h-3 w-3" />
            </button>
          </span>
        ))}
        <input
          ref={inputRef}
          className="flex-1 min-w-[120px] bg-transparent text-sm outline-none placeholder:text-muted-foreground"
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
