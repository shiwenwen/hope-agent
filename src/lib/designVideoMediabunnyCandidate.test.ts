// @vitest-environment jsdom
import { Blob as NodeBlob } from "node:buffer"
import { afterEach, beforeEach, expect, test, vi } from "vitest"
import { BufferSource, EncodedVideoPacketSource, Input, MP4 } from "mediabunny"
import { exportVideoMediabunnyCandidate as exportVideo } from "./designVideoMediabunnyCandidate"

vi.mock("html2canvas", () => ({ default: vi.fn(async () => document.createElement("canvas")) }))

const avcc = new Uint8Array([
  1, 0x42, 0, 0x1e, 0xff, 0xe1, 0, 9, 0x67, 0x42, 0, 0x1e, 0xe9, 1, 0x40, 0x7b, 0x20, 1, 0, 4, 0x68,
  0xce, 6, 0xe2,
])
const closed = vi.fn()
const frameClosed = vi.fn()
const removed = vi.fn()
const seeks: number[] = []
let encoderFailure = false

class SyntheticChunk {
  readonly byteLength = 6
  readonly duration = 500_000
  readonly timestamp: number
  readonly type: "key" | "delta"
  constructor(timestamp: number, type: "key" | "delta") {
    this.timestamp = timestamp
    this.type = type
  }
  copyTo(destination: Uint8Array) {
    destination.set([0, 0, 0, 2, this.type === "key" ? 0x65 : 0x41, 0x80])
  }
}

beforeEach(() => {
  vi.stubGlobal("Blob", NodeBlob)
  vi.stubGlobal("EncodedVideoChunk", SyntheticChunk)
  vi.stubGlobal("EncodedAudioChunk", class {})
  vi.stubGlobal(
    "VideoFrame",
    class {
      readonly init: { timestamp: number }
      constructor(_canvas: unknown, init: { timestamp: number }) {
        this.init = init
      }
      close = frameClosed
    },
  )
  vi.stubGlobal(
    "VideoEncoder",
    class {
      static async isConfigSupported() {
        return { supported: true }
      }
      readonly callbacks: {
        output: (chunk: SyntheticChunk, metadata: unknown) => void
        error: (error: Error) => void
      }
      constructor(callbacks: {
        output: (chunk: SyntheticChunk, metadata: unknown) => void
        error: (error: Error) => void
      }) {
        this.callbacks = callbacks
      }
      configure() {}
      encode(frame: { init: { timestamp: number } }, options: { keyFrame: boolean }) {
        if (encoderFailure) {
          this.callbacks.error(new Error("synthetic encoder failure"))
          return
        }
        this.callbacks.output(
          new SyntheticChunk(frame.init.timestamp, options.keyFrame ? "key" : "delta"),
          {
            decoderConfig: {
              codec: "avc1.42001e",
              codedWidth: 16,
              codedHeight: 16,
              description: avcc,
            },
          },
        )
      }
      async flush() {}
      close = closed
    },
  )
  const originalCreate = document.createElement.bind(document)
  const fixture = document.implementation.createHTMLDocument("synthetic video")
  fixture.body.innerHTML = '<div class="ds-stage">synthetic</div>'
  vi.spyOn(document, "createElement").mockImplementation((tag: string) => {
    const element = originalCreate(tag)
    if (tag === "iframe") {
      Object.defineProperties(element, {
        contentDocument: { value: fixture },
        contentWindow: {
          value: { __dsDuration: () => 1000, __dsSeek: (ms: number) => seeks.push(ms) },
        },
        src: { set: () => queueMicrotask(() => element.dispatchEvent(new Event("load"))) },
      })
      element.remove = removed
    }
    return element
  })
  vi.spyOn(document.body, "appendChild").mockImplementation((element) => element)
  vi.stubGlobal(
    "URL",
    class extends URL {
      static createObjectURL() {
        return "blob:synthetic-video"
      }
      static revokeObjectURL() {}
    },
  )
  seeks.length = 0
  encoderFailure = false
})

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
  vi.clearAllMocks()
})

test("finalizes real MP4 bytes after ordered asynchronous writes", async () => {
  const actualAdd = EncodedVideoPacketSource.prototype.add
  let pending = 0
  let maximum = 0
  vi.spyOn(EncodedVideoPacketSource.prototype, "add").mockImplementation(async function (
    this: EncodedVideoPacketSource,
    ...args
  ) {
    pending++
    maximum = Math.max(maximum, pending)
    await Promise.resolve()
    try {
      return await actualAdd.apply(this, args)
    } finally {
      pending--
    }
  })
  const progress = vi.fn()
  const blob = await exportVideo("<head></head>", 16, 16, {
    fps: 2,
    scale: 1,
    onProgress: progress,
  })
  expect(blob.type).toBe("video/mp4")
  const input = new Input({
    source: new BufferSource(Uint8Array.from(new Uint8Array(await blob.arrayBuffer()))),
    formats: [MP4],
  })
  try {
    const track = await input.getPrimaryVideoTrack()
    expect(await track?.getCodec()).toBe("avc")
    const stats = await track?.computePacketStats()
    expect(stats?.packetCount).toBe(2)
    expect(seeks).toEqual([0, 500])
    expect(maximum).toBe(1)
    expect(progress).toHaveBeenLastCalledWith(2, 2)
    expect(closed).toHaveBeenCalledOnce()
    expect(frameClosed).toHaveBeenCalledTimes(2)
    expect(removed).toHaveBeenCalledOnce()
  } finally {
    input.dispose()
  }
})

test("rejects a failed asynchronous packet write and releases resources", async () => {
  vi.spyOn(EncodedVideoPacketSource.prototype, "add").mockRejectedValue(
    new Error("synthetic write failure"),
  )
  const progress = vi.fn()
  await expect(
    exportVideo("<head></head>", 16, 16, { fps: 2, onProgress: progress }),
  ).rejects.toThrow("synthetic write failure")
  expect(progress).not.toHaveBeenCalled()
  expect(closed).toHaveBeenCalledOnce()
  expect(frameClosed).toHaveBeenCalledOnce()
  expect(removed).toHaveBeenCalledOnce()
})

test("reports asynchronous encoder failure through the export promise", async () => {
  encoderFailure = true
  await expect(exportVideo("<head></head>", 16, 16, { fps: 2 })).rejects.toThrow(
    "synthetic encoder failure",
  )
  expect(closed).toHaveBeenCalledOnce()
  expect(removed).toHaveBeenCalledOnce()
})
