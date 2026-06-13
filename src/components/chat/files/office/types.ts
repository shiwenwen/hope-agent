/** Shared props for the per-format office rich-render views. */
export interface OfficeViewProps {
  /** Raw bytes of the office file (already fetched by `OfficeRichPreview`). */
  data: ArrayBuffer
  /**
   * Called when rich rendering fails (corrupt file, unsupported feature, lib
   * throw) — the caller then falls back to the backend's text extraction.
   */
  onError: (error: unknown) => void
}
