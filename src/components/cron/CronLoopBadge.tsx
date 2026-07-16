import { Trans } from "react-i18next"

export default function CronLoopBadge() {
  return (
    <span className="inline-flex shrink-0 items-center rounded-md bg-sky-500/10 px-1.5 py-0.5 text-[9px] font-semibold leading-none text-sky-700 dark:text-sky-300">
      <Trans i18nKey="workspace.loop.title" />
    </span>
  )
}
