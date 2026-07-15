export function shouldSubmitProjectInstructions(
  content: string,
  loadedContent: string,
  workingDir: string,
  initialWorkingDir: string | null | undefined,
): boolean {
  return content !== loadedContent || workingDir.trim() !== (initialWorkingDir ?? "").trim()
}
