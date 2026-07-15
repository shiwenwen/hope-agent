const OPEN_URL_REVOKE_DELAY_MS = 60_000
const DOWNLOAD_URL_REVOKE_DELAY_MS = 1_000

function clickObjectUrl(file: File, download: boolean): void {
  const url = URL.createObjectURL(file)
  const anchor = document.createElement("a")
  anchor.href = url
  anchor.rel = "noopener"
  if (download) {
    anchor.download = file.name
  } else {
    anchor.target = "_blank"
  }
  document.body.appendChild(anchor)
  anchor.click()
  document.body.removeChild(anchor)
  window.setTimeout(
    () => URL.revokeObjectURL(url),
    download ? DOWNLOAD_URL_REVOKE_DELAY_MS : OPEN_URL_REVOKE_DELAY_MS,
  )
}

export function openStagedFile(file: File): void {
  clickObjectUrl(file, false)
}

export function downloadStagedFile(file: File): void {
  clickObjectUrl(file, true)
}
