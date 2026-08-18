/**
 * Shell shared by every title-bar icon action, so one-shot actions (search,
 * export, handover) and stateful toggles (incognito, session status, terminal,
 * workbench) sit on one rhythm. Only the toggles add the active fill.
 */
export const TITLE_BAR_ICON_BUTTON =
  "flex h-7 w-7 items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-secondary/40 hover:text-foreground"

export const TITLE_BAR_ICON_BUTTON_ACTIVE = "bg-secondary text-foreground hover:bg-secondary/85"
