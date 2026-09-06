import { useTranslation } from "react-i18next"
import type { AskUserQuestion, AskUserQuestionGroup } from "./AskUserQuestionBlock"

/** Describe the effective server deadline, never the model's requested policy. */
export function AskUserWaitHint({ group }: { group: AskUserQuestionGroup }) {
  const { t } = useTranslation()
  const hasFallback = !group.ownerResponse && group.questions.some((q) => q.defaultValues?.length)
  const key = !group.timeoutAt
    ? "planMode.question.waitForever"
    : hasFallback
      ? "planMode.question.timeoutWithFallback"
      : "planMode.question.timeoutWithoutFallback"
  return <p className="text-xs text-muted-foreground">{t(key)}</p>
}

export function AskUserFallbackHint({
  group,
  question,
}: {
  group: AskUserQuestionGroup
  question: AskUserQuestion
}) {
  const { t } = useTranslation()
  if (!group.timeoutAt || group.ownerResponse || !question.defaultValues?.length) return null
  const values = question.defaultValues.map((value) => {
    const label = question.options.find((option) => option.value === value)?.label
    if (!label) return value
    return typeof label === "string"
      ? label
      : t(label.key, { ...label.params, defaultValue: label.fallback || label.key })
  })
  return (
    <p className="text-xs text-muted-foreground break-words">
      {t("planMode.question.fallback")}: {values.join(" · ")}
    </p>
  )
}
