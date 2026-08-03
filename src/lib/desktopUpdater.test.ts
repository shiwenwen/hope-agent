import { describe, expect, test, vi } from "vitest"
import type { DesktopUpdate, DesktopUpdateEvent } from "./desktopUpdater"
import {
  getDownloadStatus,
  installUpdate,
  isDesktopUpdateInstalled,
  setPendingUpdate,
  silentDownload,
} from "./desktopUpdater"

vi.mock("@/lib/logger", () => ({
  logger: {
    info: vi.fn(),
    warn: vi.fn(),
    error: vi.fn(),
  },
}))

function deferred() {
  let resolve!: () => void
  const promise = new Promise<void>((done) => {
    resolve = done
  })
  return { promise, resolve }
}

function fakeUpdate(version: string): DesktopUpdate & {
  emit: (event: DesktopUpdateEvent) => void
  downloaded: () => boolean
} {
  let onEvent: ((event: DesktopUpdateEvent) => void) | undefined
  let isDownloaded = false
  const gate = deferred()

  return {
    currentVersion: "0.26.0",
    version,
    download: vi.fn(async (listener) => {
      onEvent = listener
      await gate.promise
      isDownloaded = true
    }),
    install: vi.fn(async () => {
      if (!isDownloaded) throw new Error("Update.install called before Update.download")
    }),
    downloadAndInstall: vi.fn(),
    close: vi.fn(async () => {}),
    emit: (event) => {
      onEvent?.(event)
      if (event.event === "Finished") gate.resolve()
    },
    downloaded: () => isDownloaded,
  }
}

describe("desktop updater download coordination", () => {
  test("replays silent-download progress and installs the same Update resource", async () => {
    const update = fakeUpdate("0.27.0")
    await setPendingUpdate(update)

    const backgroundDownload = silentDownload(update)
    await vi.waitFor(() => expect(update.download).toHaveBeenCalledOnce())
    update.emit({ event: "Started", data: { contentLength: 100 } })
    update.emit({ event: "Progress", data: { chunkLength: 40 } })

    const events: DesktopUpdateEvent[] = []
    const installing = installUpdate(update, (event) => events.push(event))

    expect(events).toEqual([
      { event: "Started", data: { contentLength: 100 } },
      { event: "Progress", data: { chunkLength: 40 } },
    ])

    update.emit({ event: "Progress", data: { chunkLength: 60 } })
    update.emit({ event: "Finished" })
    await Promise.all([backgroundDownload, installing])

    expect(update.download).toHaveBeenCalledOnce()
    expect(update.install).toHaveBeenCalledOnce()
    expect(events.at(-1)).toEqual({ event: "Finished" })
    expect(getDownloadStatus()).toBe("idle")
    expect(isDesktopUpdateInstalled(update)).toBe(true)
  })

  test("reuses the downloaded resource returned by an earlier check", async () => {
    const downloadedUpdate = fakeUpdate("0.28.0")
    await setPendingUpdate(downloadedUpdate)
    const download = silentDownload(downloadedUpdate)
    await vi.waitFor(() => expect(downloadedUpdate.download).toHaveBeenCalledOnce())
    downloadedUpdate.emit({ event: "Finished" })
    await download

    const duplicateCheck = fakeUpdate("0.28.0")
    const installableUpdate = await setPendingUpdate(duplicateCheck)

    expect(installableUpdate).toBe(downloadedUpdate)
    expect(duplicateCheck.close).toHaveBeenCalledOnce()
    await installUpdate(installableUpdate!)
    expect(downloadedUpdate.download).toHaveBeenCalledOnce()
    expect(downloadedUpdate.install).toHaveBeenCalledOnce()
  })

  test("downloads a second Update object even when it describes the same release", async () => {
    const firstResource = fakeUpdate("0.29.0")
    await setPendingUpdate(firstResource)
    const firstDownload = silentDownload(firstResource)
    await vi.waitFor(() => expect(firstResource.download).toHaveBeenCalledOnce())
    firstResource.emit({ event: "Finished" })
    await firstDownload

    const secondResource = fakeUpdate("0.29.0")
    const installing = installUpdate(secondResource)
    await vi.waitFor(() => expect(secondResource.download).toHaveBeenCalledOnce())
    secondResource.emit({ event: "Finished" })
    await installing

    expect(secondResource.downloaded()).toBe(true)
    expect(secondResource.install).toHaveBeenCalledOnce()
  })

  test("does not borrow an in-flight download from a different release", async () => {
    const oldUpdate = fakeUpdate("0.30.0")
    await setPendingUpdate(oldUpdate)
    const oldDownload = silentDownload(oldUpdate)
    await vi.waitFor(() => expect(oldUpdate.download).toHaveBeenCalledOnce())

    const newUpdate = fakeUpdate("0.31.0")
    await setPendingUpdate(newUpdate)
    const installing = installUpdate(newUpdate)
    await vi.waitFor(() => expect(newUpdate.download).toHaveBeenCalledOnce())
    newUpdate.emit({ event: "Finished" })
    await installing

    expect(newUpdate.downloaded()).toBe(true)
    expect(newUpdate.install).toHaveBeenCalledOnce()

    oldUpdate.emit({ event: "Finished" })
    await oldDownload
  })

  test("retains a silent download even when it is not surfaced as pending", async () => {
    const firstResource = fakeUpdate("0.32.0")
    const firstDownload = silentDownload(firstResource)
    await vi.waitFor(() => expect(firstResource.download).toHaveBeenCalledOnce())
    firstResource.emit({ event: "Finished" })
    await firstDownload

    const duplicateCheck = fakeUpdate("0.32.0")
    await silentDownload(duplicateCheck)

    expect(duplicateCheck.close).toHaveBeenCalledOnce()
    expect(duplicateCheck.download).not.toHaveBeenCalled()
    expect(firstResource.download).toHaveBeenCalledOnce()
  })

  test("coalesces concurrent silent downloads from separate check resources", async () => {
    const firstResource = fakeUpdate("0.32.1")
    const duplicateCheck = fakeUpdate("0.32.1")
    const firstDownload = silentDownload(firstResource)
    const duplicateDownload = silentDownload(duplicateCheck)

    await vi.waitFor(() => expect(firstResource.download).toHaveBeenCalledOnce())
    firstResource.emit({ event: "Finished" })
    await Promise.all([firstDownload, duplicateDownload])

    expect(duplicateCheck.close).toHaveBeenCalledOnce()
    expect(duplicateCheck.download).not.toHaveBeenCalled()
  })

  test("shares the installed state and never reinstalls a consumed resource", async () => {
    const update = fakeUpdate("0.33.0")
    await setPendingUpdate(update)
    const download = silentDownload(update)
    await vi.waitFor(() => expect(update.download).toHaveBeenCalledOnce())
    update.emit({ event: "Finished" })
    await download
    await installUpdate(update)

    expect(isDesktopUpdateInstalled(update)).toBe(true)
    await installUpdate(update)
    await silentDownload(update)
    expect(update.install).toHaveBeenCalledOnce()
    expect(update.download).toHaveBeenCalledOnce()

    const duplicateCheck = fakeUpdate("0.33.0")
    expect(await setPendingUpdate(duplicateCheck)).toBe(update)
    expect(duplicateCheck.close).toHaveBeenCalledOnce()
  })
})
