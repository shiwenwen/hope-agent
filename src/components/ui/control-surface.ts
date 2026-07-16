export type ControlSurface = "default" | "embedded"

/** Shared flat surface for standard form controls. */
export const FLAT_CONTROL_SURFACE_CLASS =
  "rounded-lg border border-border/60 bg-background/40 text-foreground shadow-none transition-colors hover:bg-muted/40 focus:outline-none disabled:cursor-not-allowed disabled:opacity-50 forced-colors:border-[CanvasText]"

/** Borderless surface for controls whose visual boundary is owned by an outer shell. */
export const EMBEDDED_CONTROL_SURFACE_CLASS =
  "rounded-none border-0 bg-transparent shadow-none"
