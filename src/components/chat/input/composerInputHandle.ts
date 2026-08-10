export interface ComposerInputHandle {
  focus: () => void
  getValue: () => string
  insertNewline: () => void
  getSelectionRange: () => { start: number; end: number }
  setSelectionRange: (start: number, end: number) => void
}
