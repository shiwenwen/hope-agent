export interface ComposerInputHandle {
  focus: () => void
  getValue: () => string
  getSelectionRange: () => { start: number; end: number }
  setSelectionRange: (start: number, end: number) => void
}
