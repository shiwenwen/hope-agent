/**
 * SDK-11 的离线 MP4 迁移候选，仅由合成测试引用。
 * 当前生产导出保持 designVideo.ts；实际平台播放验收通过后才决定切换。
 *
 * 关键：**零重依赖、纯客户端**——不捆 headless Chrome / FFmpeg（对症旧路线的重管线）。
 * 用**确定性时钟 harness**（patch rAF / performance.now / Date.now + Web Animations API
 * getAnimations 定格）把 motion 产物的 CSS/JS 动画逐帧定格，`html2canvas` 抓帧，
 * **WebCodecs VideoEncoder（H.264）** 编码 + `Mediabunny` 封装成 MP4。此处只验证离线封装；平台 WebCodecs/播放兼容性尚待验收。
 */

import html2canvas from "html2canvas"
import {
  Output,
  Mp4OutputFormat,
  BufferTarget,
  EncodedVideoPacketSource,
  EncodedPacket,
} from "mediabunny"

export interface VideoOpts {
  /** 栅格化倍率（清晰度），钳 [1,3]（视频逐帧，倍率越大越慢）。默认 1.5。 */
  scale?: number
  /** 帧率。默认 30。 */
  fps?: number
  /** 目标码率（bps）。缺省按分辨率 × 帧率自适应。 */
  bitrate?: number
  /** 最长时长（秒），钳 [1,300]。默认 120。 */
  maxDurationSec?: number
  onProgress?: (done: number, total: number) => void
}

/** 是否具备客户端视频编码能力（WebCodecs）。 */
export function videoExportSupported(): boolean {
  return typeof (globalThis as unknown as { VideoEncoder?: unknown }).VideoEncoder !== "undefined"
}

/** 按分辨率挑 H.264 level（Baseline），避免高分辨率被低 level 拒绝编码。 */
function h264Codec(w: number, h: number): string {
  const mb = Math.ceil(w / 16) * Math.ceil(h / 16)
  if (mb <= 3600) return "avc1.42001f" // ≤ 720p → level 3.1
  if (mb <= 8192) return "avc1.420028" // ≤ 1080p → level 4.0
  if (mb <= 22080) return "avc1.420032" // ≤ 4K → level 5.0
  return "avc1.420033" // level 5.1
}

/**
 * 注入确定性时钟 harness：patch rAF/performance.now/Date.now 为虚拟时钟（默认冻结在 0），
 * 暴露 `__dsSeek(ms)`（推进虚拟时钟 + 冲刷 rAF 回调 + WAAPI 定格）与 `__dsDuration()`
 * （读 `.ds-stage[data-ds-duration]` 或 WAAPI 最长动画结束时间）。必须在 body 用户脚本前运行，
 * 故插到 `</head>` 前。
 */
const HARNESS = `<script>
(function(){
  var vt=0, cbs=[], nid=1;
  try{Object.defineProperty(performance,'now',{value:function(){return vt},configurable:true});}
  catch(e){try{performance.now=function(){return vt};}catch(_){}}
  try{Date.now=function(){return vt};}catch(e){}
  window.requestAnimationFrame=function(fn){var id=nid++;cbs.push([id,fn]);return id;};
  window.cancelAnimationFrame=function(id){cbs=cbs.filter(function(c){return c[0]!==id;});};
  window.__dsSeek=function(ms){
    vt=ms;
    var p=cbs;cbs=[];
    p.forEach(function(c){try{c[1](ms);}catch(e){}});
    try{(document.getAnimations?document.getAnimations():[]).forEach(function(a){
      try{a.pause();a.currentTime=ms;}catch(e){}});}catch(e){}
  };
  window.__dsDuration=function(){
    var s=document.querySelector('.ds-stage');
    var d=s&&s.getAttribute('data-ds-duration');
    if(d&&+d>0)return +d;
    var max=0;
    try{(document.getAnimations?document.getAnimations():[]).forEach(function(a){
      try{var ct=a.effect&&a.effect.getComputedTiming?a.effect.getComputedTiming():null;
      if(ct&&isFinite(ct.endTime))max=Math.max(max,ct.endTime);}catch(e){}});}catch(e){}
    return max;
  };
})();
</script>`

function injectHarness(html: string): string {
  const i = html.indexOf("</head>")
  return i >= 0 ? html.slice(0, i) + HARNESS + html.slice(i) : HARNESS + html
}

/** 偶数化（H.264 要求宽高为偶数）。 */
const even = (n: number) => Math.max(2, Math.round(n / 2) * 2)

interface Sized {
  doc: Document
  win: Window & { __dsSeek?: (ms: number) => void; __dsDuration?: () => number }
  cleanup: () => void
}

/** 把（注入 harness 的）HTML 载入固定尺寸的离屏同源 iframe。 */
async function renderSized(html: string, width: number, height: number): Promise<Sized> {
  const url = URL.createObjectURL(new Blob([html], { type: "text/html" }))
  const iframe = document.createElement("iframe")
  iframe.setAttribute("aria-hidden", "true")
  iframe.style.cssText = `position:fixed;left:-99999px;top:0;width:${width}px;height:${height}px;border:0;background:#000;visibility:hidden`
  document.body.appendChild(iframe)
  try {
    await new Promise<void>((resolve, reject) => {
      iframe.onload = () => resolve()
      iframe.onerror = () => reject(new Error("video iframe failed to load"))
      iframe.src = url
    })
  } catch (e) {
    iframe.remove()
    URL.revokeObjectURL(url)
    throw e
  }
  await new Promise((r) => setTimeout(r, 200))
  const doc = iframe.contentDocument
  const win = iframe.contentWindow
  if (!doc || !win) {
    iframe.remove()
    URL.revokeObjectURL(url)
    throw new Error("video iframe has no document")
  }
  return {
    doc,
    win: win as Sized["win"],
    cleanup: () => {
      iframe.remove()
      URL.revokeObjectURL(url)
    },
  }
}

/**
 * 把 motion 产物 HTML 导出为 MP4。逐帧：`__dsSeek(t)` 定格 → `html2canvas` 抓帧 →
 * WebCodecs 编码 → Mediabunny 封装。时长取 `data-ds-duration` / WAAPI / 6s 兜底，钳 [1s,30s]。
 */
export async function exportVideoMediabunnyCandidate(
  html: string,
  vw?: number,
  vh?: number,
  opts?: VideoOpts,
): Promise<Blob> {
  const G = globalThis as unknown as {
    VideoEncoder?: {
      new (init: { output: (c: unknown, m: unknown) => void; error: (e: unknown) => void }): {
        configure: (c: unknown) => void
        encode: (f: unknown, o?: { keyFrame?: boolean }) => void
        flush: () => Promise<void>
        close: () => void
      }
      isConfigSupported?: (c: unknown) => Promise<{ supported?: boolean }>
    }
    VideoFrame?: new (src: CanvasImageSource, init: { timestamp: number }) => { close: () => void }
  }
  if (!G.VideoEncoder || !G.VideoFrame) {
    throw new Error("video export requires WebCodecs (desktop app / Chrome / Edge)")
  }
  const width = vw && vw > 0 ? vw : 1280
  const height = vh && vh > 0 ? vh : 720
  const scale = Math.min(3, Math.max(1, opts?.scale ?? 1.5))
  const fps = opts?.fps ?? 30
  const W = even(width * scale)
  const H = even(height * scale)
  // 码率：缺省按分辨率 × 帧率自适应（~0.12 bit/px/frame），钳 [3,24] Mbps；6Mbps 固定值对
  // 1080p 偏低。
  const bitrate =
    opts?.bitrate ?? Math.round(Math.min(24_000_000, Math.max(3_000_000, W * H * fps * 0.12)))

  const config = { codec: h264Codec(W, H), width: W, height: H, bitrate, framerate: fps }
  const sup = await G.VideoEncoder.isConfigSupported?.(config)
  if (sup && sup.supported === false) {
    throw new Error("H.264 encoding is not supported on this platform")
  }

  const h = await renderSized(injectHarness(html), width, height)
  try {
    let durMs = 6000
    try {
      const d = h.win.__dsDuration?.()
      if (d && d > 0) durMs = d
    } catch {
      /* default */
    }
    const maxDurMs = Math.min(300, Math.max(1, opts?.maxDurationSec ?? 120)) * 1000
    durMs = Math.min(maxDurMs, Math.max(1000, durMs))
    const totalFrames = Math.max(1, Math.round((durMs / 1000) * fps))

    const target = new BufferTarget()
    const output = new Output({
      target,
      format: new Mp4OutputFormat({ fastStart: "in-memory" }),
    })
    const source = new EncodedVideoPacketSource("avc")
    output.addVideoTrack(source, { frameRate: fps })
    let encodeError: Error | undefined
    let writes = Promise.resolve()
    const encoder = new G.VideoEncoder({
      output: (chunk: unknown, meta: unknown) => {
        try {
          const packet = EncodedPacket.fromEncodedChunk(chunk as EncodedVideoChunk)
          writes = writes
            .then(async () => {
              if (!encodeError) await source.add(packet, meta as EncodedVideoChunkMetadata)
            })
            .catch((error: unknown) => {
              encodeError = error instanceof Error ? error : new Error(String(error))
            })
        } catch (error) {
          encodeError = error instanceof Error ? error : new Error(String(error))
        }
      },
      error: (error: unknown) => {
        encodeError = error instanceof Error ? error : new Error(String(error))
      },
    })
    try {
      await output.start()
      encoder.configure(config)

      const stage = (h.doc.querySelector(".ds-stage") as HTMLElement | null) ?? h.doc.body
      for (let i = 0; i < totalFrames; i++) {
        const t = (i / fps) * 1000
        try {
          h.win.__dsSeek?.(t)
        } catch {
          /* ignore per-frame seek errors */
        }
        await new Promise((r) => setTimeout(r, 0))
        const canvas = await html2canvas(stage, {
          backgroundColor: "#000000",
          scale,
          useCORS: true,
          logging: false,
          width,
          height,
          windowWidth: width,
          windowHeight: height,
        })
        const frame = new G.VideoFrame(canvas, { timestamp: Math.round(t * 1000) })
        try {
          encoder.encode(frame, { keyFrame: i % fps === 0 })
        } finally {
          frame.close()
        }
        // WebCodecs callbacks cannot await backpressure. Drain each captured
        // frame before producing another, keeping the asynchronous writer bounded.
        await encoder.flush()
        await writes
        if (encodeError) throw encodeError
        opts?.onProgress?.(i + 1, totalFrames)
      }
      await output.finalize()
      if (!target.buffer) throw new Error("video export did not produce an MP4")
      return new Blob([target.buffer], { type: "video/mp4" })
    } finally {
      try {
        encoder.close()
      } catch {
        /* encoder errors may already close it */
      }
      if (output.state !== "finalized") await output.cancel()
    }
  } finally {
    h.cleanup()
  }
}
