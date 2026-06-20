import { createServer } from "node:http"
import { readFileSync } from "node:fs"
import path from "node:path"
import { fileURLToPath } from "node:url"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const extensionDir = path.resolve(scriptDir, "..")
const pagesDir = path.join(extensionDir, "test-pages")

const checkMode = process.argv.includes("--check")
const mainPort = checkMode ? 0 : Number(process.env.HOPE_EXTENSION_SMOKE_MAIN_PORT || 17610)
const framePort = checkMode ? 0 : Number(process.env.HOPE_EXTENSION_SMOKE_FRAME_PORT || 17611)

const mainServer = makeServer("main")
const frameServer = makeServer("frame")

await listen(mainServer, mainPort)
await listen(frameServer, framePort)

const mainOrigin = originFor(mainServer)
const frameOrigin = originFor(frameServer)
const rootUrl = `${mainOrigin}/root.html`
const frameUrl = `${frameOrigin}/frame.html`

if (checkMode) {
  try {
    await runCheck(rootUrl, frameUrl)
    console.log("[chrome-extension:smoke-pages] ok")
  } finally {
    await closeServer(mainServer)
    await closeServer(frameServer)
  }
} else {
  console.log("[chrome-extension:smoke-pages] serving")
  console.log(`  root:        ${rootUrl}`)
  console.log(`  cross frame: ${frameUrl}`)
  console.log("")
  console.log("Manual browser smoke:")
  console.log("  1. Load the unpacked extension and install the native host.")
  console.log("  2. Open the root URL in Chrome.")
  console.log("  3. In Hope Agent, claim the tab and run browser snapshot/action/screenshot checks.")
  console.log("  4. Verify browser.status shows frame tree + matched flat sessions.")
  console.log("")
  console.log("Press Ctrl+C to stop.")
}

process.on("SIGINT", () => {
  Promise.allSettled([closeServer(mainServer), closeServer(frameServer)]).finally(() => {
    process.exit(0)
  })
})

function makeServer(kind) {
  return createServer((req, res) => {
    try {
      const url = new URL(req.url || "/", `http://${req.headers.host || "127.0.0.1"}`)
      const pathname = url.pathname === "/" ? "/root.html" : url.pathname
      const name = path.basename(pathname)
      if (!["root.html", "same-origin-frame.html", "frame.html"].includes(name)) {
        res.writeHead(404, { "content-type": "text/plain; charset=utf-8" })
        res.end("not found")
        return
      }
      if (kind === "frame" && name !== "frame.html") {
        res.writeHead(404, { "content-type": "text/plain; charset=utf-8" })
        res.end("not found")
        return
      }
      let body = readFileSync(path.join(pagesDir, name), "utf8")
      body = body.replaceAll("__CROSS_ORIGIN__", frameOriginIfReady())
      res.writeHead(200, {
        "cache-control": "no-store",
        "content-type": "text/html; charset=utf-8",
      })
      res.end(body)
    } catch (error) {
      res.writeHead(500, { "content-type": "text/plain; charset=utf-8" })
      res.end(error instanceof Error ? error.message : String(error))
    }
  })
}

async function runCheck(rootUrl, frameUrl) {
  const root = await fetchText(rootUrl)
  assertIncludes(root, 'data-hope-smoke-page="root"', "root marker")
  assertIncludes(root, frameUrl, "cross-origin frame URL")
  assertIncludes(root, 'id="same-origin-frame"', "same-origin frame")
  assertIncludes(root, 'id="cross-origin-frame"', "cross-origin frame")

  const sameFrame = await fetchText(`${mainOrigin}/same-origin-frame.html`)
  assertIncludes(sameFrame, 'data-hope-smoke-page="same-origin-frame"', "same-origin marker")
  assertIncludes(sameFrame, "Same Frame Drag Source", "same-origin drag source")

  const crossFrame = await fetchText(frameUrl)
  assertIncludes(crossFrame, 'data-hope-smoke-page="cross-origin-frame"', "cross-origin marker")
  assertIncludes(crossFrame, "Cross Frame Drag Source", "cross-origin drag source")
  assertIncludes(crossFrame, "Cross Frame Crop Target", "cross-origin crop target")
}

async function fetchText(url) {
  const response = await fetch(url)
  if (!response.ok) {
    throw new Error(`${url} returned HTTP ${response.status}`)
  }
  return response.text()
}

function assertIncludes(text, needle, label) {
  if (!text.includes(needle)) {
    throw new Error(`missing ${label}: ${needle}`)
  }
}

function listen(server, port) {
  return new Promise((resolve, reject) => {
    server.once("error", reject)
    server.listen(port, "127.0.0.1", () => {
      server.off("error", reject)
      resolve()
    })
  })
}

function closeServer(server) {
  return new Promise((resolve, reject) => {
    server.close((error) => (error ? reject(error) : resolve()))
  })
}

function originFor(server) {
  const address = server.address()
  if (!address || typeof address === "string") {
    throw new Error("server did not bind to an IPv4 address")
  }
  return `http://127.0.0.1:${address.port}`
}

function frameOriginIfReady() {
  try {
    return originFor(frameServer)
  } catch {
    return "http://127.0.0.1:17611"
  }
}
