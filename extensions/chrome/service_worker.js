// @ts-check

const HOST_NAME = "com.hope_agent.chrome"
const PROTOCOL_VERSION = 1
const MAX_DIRECT_RESPONSE_BYTES = 768 * 1024
const RESPONSE_BLOB_CHUNK_BYTES = 192 * 1024

/** @type {any | null} */
let nativePort = null
let nativeConnected = false
let nextMessageId = 1
let nextBlobId = 1
/** @type {Map<string, { resolve: (value: unknown) => void, reject: (reason: unknown) => void, timer: number }>} */
const pendingNative = new Map()

// Native-host reconnect resilience. The extension is the ONLY side that can
// (re)initiate the native connection — Chrome owns the host process lifecycle
// via connectNative, and the desktop app's broker cannot dial in. So when the
// app / broker restarts (frequent in dev) the port drops and nothing recovers
// unless we retry here. Two complementary mechanisms: exponential backoff on
// disconnect (fast recovery while the SW is alive) + a periodic alarm that
// wakes the SW and reconnects even after MV3 evicts the idle worker.
/** @type {number | null} */
let reconnectTimer = null
let reconnectAttempts = 0
const RECONNECT_BASE_MS = 1000
const RECONNECT_MAX_MS = 30000
const KEEPALIVE_ALARM = "ha-native-keepalive"

function scheduleReconnect() {
  if (reconnectTimer || nativePort) return
  // Cap the exponent: during a long broker outage reconnectAttempts keeps
  // growing (it only resets on a successful inbound message), and an uncapped
  // `2 ** n` would reach Infinity. The delay is clamped to RECONNECT_MAX_MS
  // regardless, so 2**10 (≫ the cap) is already well past saturation.
  const delay = Math.min(RECONNECT_MAX_MS, RECONNECT_BASE_MS * 2 ** Math.min(reconnectAttempts, 10))
  reconnectAttempts++
  reconnectTimer = setTimeout(() => {
    reconnectTimer = null
    try {
      ensureNativePort()
    } catch {
      scheduleReconnect()
    }
  }, delay)
}

function ensureKeepaliveAlarm() {
  try {
    chrome.alarms?.create(KEEPALIVE_ALARM, { periodInMinutes: 0.5 })
  } catch (error) {
    console.debug("Hope Agent keepalive alarm create failed", error)
  }
}

// Created at top-level so the alarm is (re)registered on every SW load, and in
// onInstalled/onStartup below for the install / browser-start lifecycle.
ensureKeepaliveAlarm()

chrome.alarms?.onAlarm.addListener((alarm) => {
  if (alarm.name !== KEEPALIVE_ALARM) return
  if (nativeConnected && nativePort) return
  try {
    ensureNativePort()
  } catch (error) {
    console.debug("Hope Agent keepalive reconnect failed", error)
  }
})
/** @type {Set<number>} */
const attachedDebugTabs = new Set()
/** @type {Set<number>} */
const flatSessionTabs = new Set()
/** @type {Map<number, Map<string, any>>} */
const flatSessionsByTab = new Map()
/** @type {Map<number, string>} */
const overlayTabs = new Map()
/** @type {Set<number>} */
const managedDownloads = new Set()
/** @type {{ console: ObserveEntry[], network: ObserveEntry[], pageErrors: ObserveEntry[], downloads: ObserveEntry[] }} */
const observeBuffers = {
  console: [],
  network: [],
  pageErrors: [],
  downloads: [],
}
const OBSERVE_RING_CAPACITY = 500

/**
 * @typedef {{ at: number, level: string, text: string, url?: string, tabId?: number }} ObserveEntry
 */

chrome.runtime.onInstalled.addListener(() => {
  ensureKeepaliveAlarm()
  ensureNativePort()
})

chrome.runtime.onStartup.addListener(() => {
  ensureKeepaliveAlarm()
  ensureNativePort()
})

chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  handleExtensionMessage(message, sender)
    .then((result) => sendResponse({ ok: true, result }))
    .catch((error) => sendResponse({ ok: false, error: serializeError(error) }))
  return true
})

chrome.debugger.onEvent.addListener((source, method, params) => {
  if (!Number.isInteger(source.tabId)) return
  handleFlatSessionEvent(source.tabId, method, params || {})
  handleDebuggerEvent(source, method, params || {})
})

chrome.debugger.onDetach.addListener((source) => {
  if (Number.isInteger(source.tabId)) {
    attachedDebugTabs.delete(source.tabId)
    flatSessionTabs.delete(source.tabId)
    flatSessionsByTab.delete(source.tabId)
  }
})

chrome.tabs.onUpdated.addListener((tabId, changeInfo) => {
  if (changeInfo.status !== "complete" || !overlayTabs.has(tabId)) return
  showOverlay(tabId, overlayTabs.get(tabId)).catch((error) => {
    console.debug("Hope Agent overlay reinject failed", error)
  })
})

chrome.tabs.onRemoved.addListener((tabId) => {
  overlayTabs.delete(tabId)
  attachedDebugTabs.delete(tabId)
  flatSessionTabs.delete(tabId)
  flatSessionsByTab.delete(tabId)
  // Reap managed download ids that finished without a terminal onChanged delta
  // (e.g. tab closed mid-download), so the set doesn't grow unbounded.
  void pruneManagedDownloads()
})

if (chrome.downloads) {
  chrome.downloads.onCreated.addListener((item) => {
    if (isHopeControlledDownload(item)) {
      managedDownloads.add(item.id)
    }
    pushObserve("downloads", {
      at: Date.now(),
      level: "created",
      text: formatDownloadItem(item),
      url: item.finalUrl || item.url,
    })
  })

  chrome.downloads.onChanged.addListener((delta) => {
    pushObserve("downloads", {
      at: Date.now(),
      level: "changed",
      text: formatDownloadDelta(delta),
    })
    if (delta.state?.current === "complete") {
      void handleDownloadCompleted(delta.id)
    } else if (delta.state?.current === "interrupted") {
      managedDownloads.delete(delta.id)
    }
  })

  chrome.downloads.onErased.addListener((downloadId) => {
    managedDownloads.delete(downloadId)
    pushObserve("downloads", {
      at: Date.now(),
      level: "erased",
      text: `download ${downloadId} erased`,
    })
  })
}

function ensureNativePort() {
  if (nativePort) return nativePort
  try {
    nativePort = chrome.runtime.connectNative(HOST_NAME)
    // Optimistic: connectNative returns a port synchronously before the host
    // process is confirmed alive. If the host is missing/crashes, onDisconnect
    // fires shortly after and resets this. sendNative uses the port directly and
    // does not gate on this flag, so the brief window only affects status display.
    nativeConnected = true
    nativePort.onMessage.addListener((message) => {
      // Any inbound message proves the port is alive — reset backoff so the next
      // disconnect retries promptly instead of inheriting a stale long delay.
      reconnectAttempts = 0
      if (reconnectTimer) {
        clearTimeout(reconnectTimer)
        reconnectTimer = null
      }
      if (message && typeof message.id === "string" && pendingNative.has(message.id)) {
        const pending = pendingNative.get(message.id)
        pendingNative.delete(message.id)
        if (pending) {
          clearTimeout(pending.timer)
          if (message.ok === false) {
            pending.reject(message.error ?? message)
          } else {
            pending.resolve(message)
          }
        }
        return
      }
      void handleHostCommand(message)
    })
    nativePort.onDisconnect.addListener(() => {
      const err = chrome.runtime.lastError?.message || "Native host disconnected"
      nativeConnected = false
      nativePort = null
      for (const [id, pending] of pendingNative.entries()) {
        clearTimeout(pending.timer)
        pending.reject(new Error(err))
        pendingNative.delete(id)
      }
      // Auto-recover: the app/broker likely restarted. Retry with backoff so the
      // user doesn't have to reload the extension by hand.
      scheduleReconnect()
    })
    nativePort.postMessage({
      id: `startup-${Date.now()}`,
      method: "extension.hello",
      protocolVersion: PROTOCOL_VERSION,
      payload: {
        extension: "hope-agent-browser-control",
        extensionVersion: chrome.runtime.getManifest().version,
      },
    })
    return nativePort
  } catch (error) {
    nativeConnected = false
    nativePort = null
    throw error
  }
}

function sendNative(method, payload = {}, timeoutMs = 5000) {
  const port = ensureNativePort()
  const id = `ext-${Date.now()}-${nextMessageId++}`
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      pendingNative.delete(id)
      reject(new Error(`Native host method timed out: ${method}`))
    }, timeoutMs)
    pendingNative.set(id, { resolve, reject, timer })
    port.postMessage({
      id,
      method,
      protocolVersion: PROTOCOL_VERSION,
      payload,
    })
  })
}

async function handleExtensionMessage(message, sender) {
  const method = message?.method || message?.type
  const params = message?.params || message?.payload || {}
  if (method === "hope.overlay.stop") {
    return handleOverlayStop(sender)
  }
  if (method === "hope.popup.status") {
    return popupStatus()
  }
  if (method === "hope.popup.stopTab") {
    return stopTabControl(requiredTabId(params), "toolbar")
  }
  if (method === "hope.popup.stopAll") {
    return stopAllControl("toolbar")
  }
  return handleCommand(method, params)
}

async function handleHostCommand(message) {
  if (!nativePort || !message || typeof message.id !== "string") return
  try {
    const result = await handleCommand(message.method, message.params || message.payload || {})
    await postHostResponse(message.id, { id: message.id, ok: true, result })
  } catch (error) {
    await postHostResponse(message.id, { id: message.id, ok: false, error: serializeError(error) })
  }
}

async function postHostResponse(id, response) {
  const port = nativePort
  if (!port) return
  const encoded = JSON.stringify(response)
  const bytes = new TextEncoder().encode(encoded)
  if (bytes.byteLength <= MAX_DIRECT_RESPONSE_BYTES) {
    port.postMessage(response)
    return
  }

  const blobId = `response-${safeBlobIdPart(id)}-${Date.now()}-${nextBlobId++}`
  const sha256 = await sha256HexBytes(bytes)
  await postHostBlob(port, {
    blobId,
    bytes,
    sha256,
    mime: "application/json",
    purpose: "response",
  })
  port.postMessage({
    id,
    ok: true,
    type: "response.blob",
    blobId,
    totalSize: bytes.byteLength,
    sha256,
    mime: "application/json",
  })
}

async function postHostBlob(port, descriptor) {
  const { blobId, bytes, sha256, mime, purpose } = descriptor
  const totalChunks = Math.ceil(bytes.byteLength / RESPONSE_BLOB_CHUNK_BYTES)
  port.postMessage({
    method: "blob.begin",
    payload: {
      blobId,
      mime,
      purpose,
      totalSize: bytes.byteLength,
      sha256,
    },
  })
  // No per-chunk sha256: the broker verifies the whole-blob sha256 (sent in
  // blob.begin/end) and treats per-chunk hashes as optional. Hashing every
  // chunk doubled the crypto work on large captures for no added safety.
  for (let index = 0; index < totalChunks; index++) {
    const offset = index * RESPONSE_BLOB_CHUNK_BYTES
    const chunk = bytes.subarray(offset, Math.min(bytes.byteLength, offset + RESPONSE_BLOB_CHUNK_BYTES))
    port.postMessage({
      method: "blob.chunk",
      payload: {
        blobId,
        index,
        offset,
        base64: base64Bytes(chunk),
      },
    })
  }
  port.postMessage({
    method: "blob.end",
    payload: {
      blobId,
      totalChunks,
      sha256,
    },
  })
}

function safeBlobIdPart(id) {
  const safe = String(id || "response").replace(/[^A-Za-z0-9_.-]/g, "_")
  return safe.slice(0, 48) || "response"
}

async function sha256HexBytes(bytes) {
  const digest = await crypto.subtle.digest("SHA-256", bytes)
  return Array.from(new Uint8Array(digest), (byte) => byte.toString(16).padStart(2, "0")).join("")
}

function base64Bytes(bytes) {
  let binary = ""
  for (let index = 0; index < bytes.length; index += 0x8000) {
    const slice = bytes.subarray(index, index + 0x8000)
    binary += String.fromCharCode(...slice)
  }
  return btoa(binary)
}

function bytesFromBase64(base64) {
  const binary = atob(base64)
  const bytes = new Uint8Array(binary.length)
  for (let index = 0; index < binary.length; index++) {
    bytes[index] = binary.charCodeAt(index)
  }
  return bytes
}

async function handleCommand(method, params) {
  switch (method) {
    case "hello":
      return {
        extension: "hope-agent-browser-control",
        extensionVersion: chrome.runtime.getManifest().version,
        protocolVersion: PROTOCOL_VERSION,
        nativeConnected,
      }
    case "status":
      return extensionStatus()
    case "native.hello":
      return sendNative("hello", {
        extensionVersion: chrome.runtime.getManifest().version,
      })
    case "native.status":
      return sendNative("status")
    case "tabs.query":
      return (await chrome.tabs.query(params.query || {})).map(tabToPlain)
    case "tabs.create":
      return tabToPlain(await chrome.tabs.create(params))
    case "tabs.update":
      return tabToPlain(await chrome.tabs.update(requiredTabId(params), params.update || {}))
    case "tabs.remove":
      await chrome.tabs.remove(requiredTabId(params))
      return { removed: true }
    case "debugger.attach":
      await ensureDebuggerAttached(requiredTabId(params), params.version || "1.3")
      return { attached: true }
    case "debugger.detach":
      await chrome.debugger.detach({ tabId: requiredTabId(params) })
      attachedDebugTabs.delete(requiredTabId(params))
      flatSessionTabs.delete(requiredTabId(params))
      flatSessionsByTab.delete(requiredTabId(params))
      return { detached: true }
    case "debugger.sendCommand":
      return sendDebuggerCommand(params)
    case "debugger.sessions":
      return flatSessionsForTab(requiredTabId(params))
    case "frames.tree":
      return frameTreeForTab(requiredTabId(params))
    case "frames.snapshot":
      return snapshotFrames(requiredTabId(params), params.maxElements)
    case "frames.act":
      return actInFrame(
        requiredTabId(params),
        requiredFrameId(params),
        requiredString(params, "selector"),
        requiredString(params, "kind"),
        params.params || {},
      )
    case "overlay.show":
      await showOverlay(requiredTabId(params), params.label)
      return { shown: true }
    case "overlay.hide":
      await hideOverlay(requiredTabId(params))
      return { hidden: true }
    case "observe.read":
      return readObserveBuffer(params.kind, params.since, params.tabId)
    case "downloads.cancel":
      return cancelDownload(params)
    default:
      throw new Error(`Unsupported extension command: ${method}`)
  }
}

async function sendDebuggerCommand(params) {
  const command = requiredString(params, "command")
  const commandParams = params.params || {}
  const result = await chrome.debugger.sendCommand(debuggerTarget(params), command, commandParams)
  return maybeBlobBackedCdpResult(command, commandParams, result)
}

async function maybeBlobBackedCdpResult(command, commandParams, result) {
  if (!nativePort || !result || typeof result.data !== "string") return result
  const blobMeta = cdpBinaryBlobMeta(command, commandParams)
  if (!blobMeta) return result
  const bytes = bytesFromBase64(result.data)
  const blobId = `${blobMeta.purpose}-${Date.now()}-${nextBlobId++}`
  const sha256 = await sha256HexBytes(bytes)
  await postHostBlob(nativePort, {
    blobId,
    bytes,
    sha256,
    mime: blobMeta.mime,
    purpose: blobMeta.purpose,
  })
  const out = { ...result }
  delete out.data
  out.dataBlob = {
    blobId,
    totalSize: bytes.byteLength,
    sha256,
    mime: blobMeta.mime,
    purpose: blobMeta.purpose,
    encoding: "raw",
  }
  return out
}

function cdpBinaryBlobMeta(command, commandParams) {
  if (command === "Page.printToPDF") {
    return { mime: "application/pdf", purpose: "pdf" }
  }
  if (command === "Page.captureScreenshot") {
    const format = String(commandParams?.format || "png").toLowerCase()
    const mime = format === "jpeg" || format === "jpg" ? "image/jpeg" : "image/png"
    return { mime, purpose: "screenshot" }
  }
  return null
}

async function snapshotFrames(tabId, maxElements) {
  if (!chrome.scripting?.executeScript) {
    throw new Error("chrome.scripting.executeScript is unavailable")
  }
  const cappedMaxElements = normalizeMaxElements(maxElements, 160)
  const results = await chrome.scripting.executeScript({
    target: { tabId, allFrames: true },
    func: collectHopeFrameSnapshot,
    args: [cappedMaxElements],
  })
  return results
    .filter((entry) => entry && entry.result)
    .map((entry) => ({
      frameId: entry.frameId,
      documentId: entry.documentId,
      ...entry.result,
    }))
}

async function actInFrame(tabId, frameId, selector, kind, params) {
  if (!chrome.scripting?.executeScript) {
    throw new Error("chrome.scripting.executeScript is unavailable")
  }
  const results = await chrome.scripting.executeScript({
    target: { tabId, frameIds: [frameId] },
    func: performHopeFrameAction,
    args: [kind, selector, params || {}],
  })
  const result = results && results[0] && results[0].result
  if (!result || result.ok !== true) {
    throw new Error(result?.error || "Frame action did not return a result")
  }
  return result
}

/** @param {unknown} maxElements */
function collectHopeFrameSnapshot(maxElements) {
  const MAX_TEXT_LEN = 100
  /** @type {any[]} */
  const refs = []
  let refId = 0
  const cap = Math.max(1, Math.min(Number(maxElements) || 160, 300))
  const interactiveSelectors = [
    'a[href]', 'button', 'input', 'select', 'textarea',
    '[role="button"]', '[role="link"]', '[role="textbox"]',
    '[role="checkbox"]', '[role="radio"]', '[role="tab"]',
    '[role="menuitem"]', '[role="option"]', '[role="switch"]',
    '[contenteditable="true"]', '[tabindex]'
  ]
  const semanticTags = new Set([
    'h1','h2','h3','h4','h5','h6','p','li','td','th',
    'label','img','nav','main','header','footer','section',
    'article','aside','form','table','caption','figcaption'
  ])

  function canAccessTopDocument() {
    if (window.self === window.top) return true
    try {
      void window.top?.document
      return true
    } catch {
      return false
    }
  }

  /** @param {any} el */
  function isVisible(el) {
    if (!el.getBoundingClientRect) return false
    const rect = el.getBoundingClientRect()
    if (rect.width === 0 && rect.height === 0) return false
    const style = window.getComputedStyle(el)
    if (style.display === 'none' || style.visibility === 'hidden' || style.opacity === '0') {
      return false
    }
    return true
  }

  /** @param {any} el */
  function isInteractive(el) {
    return interactiveSelectors.some((sel) => {
      try { return el.matches(sel) } catch { return false }
    })
  }

  /** @param {any} el */
  function getRole(el) {
    const role = el.getAttribute('role')
    if (role) return role
    const tag = el.tagName.toLowerCase()
    const typeAttr = el.getAttribute('type')
    if (tag === 'a' && el.hasAttribute('href')) return 'link'
    if (tag === 'button') return 'button'
    if (tag === 'input') {
      if (typeAttr === 'checkbox') return 'checkbox'
      if (typeAttr === 'radio') return 'radio'
      if (typeAttr === 'submit' || typeAttr === 'button') return 'button'
      return 'textbox'
    }
    if (tag === 'textarea') return 'textbox'
    if (tag === 'select') return 'combobox'
    if (tag === 'img') return 'img'
    if (/^h[1-6]$/.test(tag)) return 'heading'
    return tag
  }

  /** @param {any} el */
  function getText(el) {
    const ariaLabel = el.getAttribute('aria-label')
    if (ariaLabel) return ariaLabel.trim().substring(0, MAX_TEXT_LEN)
    const alt = el.getAttribute('alt')
    if (alt) return alt.trim().substring(0, MAX_TEXT_LEN)
    const title = el.getAttribute('title')
    if (title && !el.children.length) return title.trim().substring(0, MAX_TEXT_LEN)
    const text = el.innerText || el.textContent || ''
    return text.trim().substring(0, MAX_TEXT_LEN)
  }

  /** @param {any} el */
  function buildUniqueSelector(el) {
    if (el.id) return '#' + CSS.escape(el.id)
    const path = []
    let current = el
    while (current && current !== document.body && path.length < 5) {
      let selector = current.tagName.toLowerCase()
      if (current.id) {
        path.unshift('#' + CSS.escape(current.id) + ' > ' + selector)
        break
      }
      if (current.className && typeof current.className === 'string') {
        const classes = current.className.trim().split(/\s+/).slice(0, 2)
        if (classes.length && classes[0]) {
          selector += '.' + classes.map((c) => CSS.escape(c)).join('.')
        }
      }
      const parent = current.parentElement
      if (parent) {
        const siblings = Array.from(parent.children).filter((c) => c.tagName === current.tagName)
        if (siblings.length > 1) selector += ':nth-of-type(' + (siblings.indexOf(current) + 1) + ')'
      }
      path.unshift(selector)
      current = current.parentElement
    }
    return path.join(' > ')
  }

  // Cap real recursion depth independently of `cap` (which only bounds emitted
  // refs): a page nesting tens of thousands of non-semantic wrappers would
  // otherwise overflow the JS stack and abort the whole snapshot.
  const MAX_WALK_DEPTH = 1000
  /** @param {any} el @param {number} depth @param {number} rawDepth */
  function walk(el, depth, rawDepth) {
    if (refId >= cap || rawDepth > MAX_WALK_DEPTH) return
    if (!el || !el.tagName) return
    if (!isVisible(el)) return
    const tag = el.tagName.toLowerCase()
    if (tag === 'iframe') return
    const interactive = isInteractive(el)
    const semantic = semanticTags.has(tag)
    if (interactive || semantic) {
      refId++
      /** @type {Record<string, unknown>} */
      const attrs = {}
      const rect = el.getBoundingClientRect()
      attrs.bounds = [rect.left, rect.top, rect.width, rect.height].map((n) => Math.round(n)).join(',')
      if (window.self !== window.top) attrs.frame = 'iframe'
      if (el.href) attrs.url = el.href
      if (el.value !== undefined && el.value !== '') attrs.value = String(el.value)
      if (el.placeholder) attrs.placeholder = el.placeholder
      if (el.name) attrs.name = el.name
      if (el.type) attrs.type = el.type
      if (el.checked !== undefined) attrs.checked = el.checked
      if (el.disabled) attrs.disabled = true
      if (el.readOnly) attrs.readonly = true
      if (tag.match(/^h[1-6]$/)) attrs.level = parseInt(tag[1], 10)
      refs.push({
        ref: refId,
        depth,
        role: getRole(el),
        text: getText(el),
        selector: buildUniqueSelector(el),
        attrs,
      })
    }
    for (const child of el.children) {
      walk(child, depth + (interactive || semantic ? 1 : 0), rawDepth + 1)
    }
  }

  if (document.body) {
    walk(document.body, 0, 0)
  }
  return {
    url: location.href,
    title: document.title,
    topAccessible: canAccessTopDocument(),
    viewport: { w: window.innerWidth, h: window.innerHeight },
    elements: refs,
    truncated: refId >= cap,
  }
}

/**
 * @param {unknown} kind
 * @param {unknown} selector
 * @param {any} params
 */
function performHopeFrameAction(kind, selector, params) {
  try {
    const el = /** @type {any} */ (document.querySelector(String(selector)))
    if (!el) throw new Error("Element not found for frame selector")
    el.scrollIntoView({ block: "center", inline: "center" })
    switch (kind) {
      case "click":
        el.click()
        return { ok: true, message: "Clicked" }
      case "double_click":
        el.dispatchEvent(new MouseEvent("dblclick", { bubbles: true, cancelable: true, view: window }))
        return { ok: true, message: "Double clicked" }
      case "hover":
        el.dispatchEvent(new MouseEvent("mouseover", { bubbles: true, cancelable: true, view: window }))
        el.dispatchEvent(new MouseEvent("mouseenter", { bubbles: true, cancelable: true, view: window }))
        return { ok: true, message: "Hovered" }
      case "fill": {
        const value = String(params?.text ?? "")
        el.focus()
        if ("value" in el) {
          const proto = Object.getPrototypeOf(el)
          const desc = Object.getOwnPropertyDescriptor(proto, "value")
          if (desc && desc.set) desc.set.call(el, value)
          else el.value = value
        } else {
          el.textContent = value
        }
        el.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: value }))
        el.dispatchEvent(new Event("change", { bubbles: true }))
        return { ok: true, message: "Filled" }
      }
      case "select": {
        const value = Array.isArray(params?.values) ? params.values[0] : params?.value
        if (value === undefined || value === null) throw new Error("act.select requires values")
        el.value = String(value)
        el.dispatchEvent(new Event("input", { bubbles: true }))
        el.dispatchEvent(new Event("change", { bubbles: true }))
        return { ok: true, message: "Selected" }
      }
      case "press": {
        const key = String(params?.key || "")
        if (!key) throw new Error("act.press requires key")
        el.focus()
        for (const type of ["keydown", "keypress", "keyup"]) {
          el.dispatchEvent(new KeyboardEvent(type, { key, bubbles: true, cancelable: true }))
        }
        return { ok: true, message: "Pressed " + key }
      }
      case "clip": {
        const rect = el.getBoundingClientRect()
        if (rect.width <= 0 || rect.height <= 0) {
          throw new Error("Element has empty bounds")
        }
        return {
          ok: true,
          message: "Clip resolved",
          url: location.href,
          title: document.title,
          clip: {
            x: Math.max(0, rect.left + window.scrollX),
            y: Math.max(0, rect.top + window.scrollY),
            width: Math.max(1, rect.width),
            height: Math.max(1, rect.height),
            scale: 1,
          },
        }
      }
      case "drag": {
        const targetSelector = String(params?.targetSelector || "")
        if (!targetSelector) throw new Error("act.drag requires targetSelector")
        const target = /** @type {any} */ (document.querySelector(targetSelector))
        if (!target) throw new Error("Drag target not found for frame selector")
        target.scrollIntoView({ block: "center", inline: "center" })
        dispatchFrameDrag(el, target)
        return { ok: true, message: "Dragged" }
      }
      default:
        throw new Error(`Unsupported frame action: ${kind}`)
    }
  } catch (error) {
    return { ok: false, error: error instanceof Error ? error.message : String(error) }
  }
}

function dispatchFrameDrag(source, target) {
  const dataTransfer = createDataTransfer()
  const sourcePoint = centerPoint(source)
  const targetPoint = centerPoint(target)
  dispatchMouseLike(source, "mousedown", sourcePoint)
  dispatchDragLike(source, "dragstart", sourcePoint, dataTransfer)
  dispatchDragLike(source, "drag", sourcePoint, dataTransfer)
  dispatchMouseLike(target, "mousemove", targetPoint)
  dispatchDragLike(target, "dragenter", targetPoint, dataTransfer)
  dispatchDragLike(target, "dragover", targetPoint, dataTransfer)
  dispatchDragLike(target, "drop", targetPoint, dataTransfer)
  dispatchDragLike(source, "dragend", targetPoint, dataTransfer)
  dispatchMouseLike(target, "mouseup", targetPoint)
}

function createDataTransfer() {
  try {
    return new DataTransfer()
  } catch {
    return undefined
  }
}

function centerPoint(el) {
  const rect = el.getBoundingClientRect()
  return {
    clientX: rect.left + rect.width / 2,
    clientY: rect.top + rect.height / 2,
  }
}

function dispatchMouseLike(el, type, point) {
  el.dispatchEvent(new MouseEvent(type, {
    bubbles: true,
    cancelable: true,
    view: window,
    clientX: point.clientX,
    clientY: point.clientY,
    button: 0,
  }))
}

function dispatchDragLike(el, type, point, dataTransfer) {
  const event = new DragEvent(type, {
    bubbles: true,
    cancelable: true,
    clientX: point.clientX,
    clientY: point.clientY,
    dataTransfer,
  })
  el.dispatchEvent(event)
}

async function handleOverlayStop(sender) {
  const tabId = sender?.tab?.id
  if (!Number.isInteger(tabId)) {
    throw new Error("Overlay stop did not include a tab id")
  }
  return stopTabControl(tabId, "overlay")
}

async function stopTabControl(tabId, source) {
  await Promise.allSettled([
    hideOverlay(tabId),
    chrome.debugger.detach({ tabId }),
  ])
  attachedDebugTabs.delete(tabId)
  flatSessionTabs.delete(tabId)
  flatSessionsByTab.delete(tabId)
  try {
    await sendNative("extension.user_stop", { tabId, source })
  } catch (error) {
    // The browser-side stop still succeeded. If Hope Agent is offline, the
    // next Core action will fail because the debugger was detached.
    console.warn("Hope Agent user_stop notification failed", error)
  }
  return { stopped: true, tabId }
}

async function stopAllControl(source) {
  const tabIds = new Set([...attachedDebugTabs, ...overlayTabs.keys()])
  const results = []
  for (const tabId of tabIds) {
    results.push(await stopTabControl(tabId, source))
  }
  return { stopped: results.length, tabs: results.map((result) => result.tabId) }
}

function popupStatus() {
  // MV3 service workers can be cold-started by the popup itself opening, before
  // onInstalled/onStartup ever ran — so the native port may never have been
  // opened and `nativeConnected` would read a stale `false`. Best-effort connect
  // here (ensureNativePort is idempotent and sets the flag optimistically) so the
  // popup reflects real connectivity. A missing host trips onDisconnect shortly
  // after, and the popup's periodic refresh corrects the display.
  try {
    ensureNativePort()
  } catch (error) {
    console.debug("Hope Agent popup status connect attempt failed", error)
  }
  return {
    nativeConnected,
    attachedTabs: attachedDebugTabs.size,
    flatSessionTabs: flatSessionTabs.size,
    flatSessions: flatSessionCount(),
    overlayTabs: overlayTabs.size,
  }
}

async function showOverlay(tabId, label) {
  // The overlay text follows the user's Chrome UI language (chrome.i18n), not
  // the desktop app's locale — the app/core can't know the browser locale, so it
  // no longer sends a label. A caller-supplied label still wins if present.
  const normalizedLabel =
    typeof label === "string" && label
      ? label
      : chrome.i18n.getMessage("overlay_controlling") || "Hope Agent is controlling this tab"
  const stopLabel = chrome.i18n.getMessage("overlay_stop") || "Stop"
  overlayTabs.set(tabId, normalizedLabel)
  await chrome.scripting.executeScript({
    target: { tabId },
    func: installHopeAgentOverlay,
    args: [normalizedLabel, stopLabel],
  })
}

async function hideOverlay(tabId) {
  overlayTabs.delete(tabId)
  await chrome.scripting.executeScript({
    target: { tabId },
    func: removeHopeAgentOverlay,
  })
}

function installHopeAgentOverlay(label, stopLabel) {
  const doc = /** @type {any} */ (globalThis).document
  const overlayId = "__hope_agent_control_overlay"
  doc.getElementById(overlayId)?.remove()

  const host = doc.createElement("div")
  host.id = overlayId
  host.style.all = "initial"
  host.style.position = "fixed"
  host.style.top = "12px"
  host.style.right = "12px"
  host.style.zIndex = "2147483647"
  host.style.pointerEvents = "auto"

  const root = host.attachShadow({ mode: "closed" })
  const style = doc.createElement("style")
  style.textContent = `
    .wrap {
      align-items: center;
      background: #111827;
      border: 1px solid rgba(255, 255, 255, 0.16);
      border-radius: 8px;
      box-shadow: 0 10px 30px rgba(0, 0, 0, 0.28);
      color: #f9fafb;
      display: flex;
      font: 12px/1.3 -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
      gap: 8px;
      max-width: min(360px, calc(100vw - 24px));
      padding: 8px 9px;
    }
    .dot {
      background: #22c55e;
      border-radius: 999px;
      box-shadow: 0 0 0 3px rgba(34, 197, 94, 0.18);
      flex: 0 0 auto;
      height: 8px;
      width: 8px;
    }
    .label {
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }
    button {
      appearance: none;
      background: #f9fafb;
      border: 0;
      border-radius: 6px;
      color: #111827;
      cursor: pointer;
      flex: 0 0 auto;
      font: 600 12px/1 -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
      padding: 6px 8px;
    }
    button:focus-visible {
      outline: 2px solid #60a5fa;
      outline-offset: 2px;
    }
  `
  const wrap = doc.createElement("div")
  wrap.className = "wrap"

  const dot = doc.createElement("span")
  dot.className = "dot"

  const text = doc.createElement("span")
  text.className = "label"
  text.textContent = label

  const button = doc.createElement("button")
  button.type = "button"
  button.textContent = stopLabel || "Stop"
  button.addEventListener("click", () => {
    doc.getElementById(overlayId)?.remove()
    try {
      chrome.runtime.sendMessage({ type: "hope.overlay.stop" })
    } catch {
      // If the extension context is gone, local removal is still the best we can do.
    }
  })

  wrap.append(dot, text, button)
  root.append(style, wrap)
  doc.documentElement.append(host)
}

function removeHopeAgentOverlay() {
  const doc = /** @type {any} */ (globalThis).document
  doc.getElementById("__hope_agent_control_overlay")?.remove()
}

async function ensureDebuggerAttached(tabId, version) {
  if (!attachedDebugTabs.has(tabId)) {
    await chrome.debugger.attach({ tabId }, version)
    attachedDebugTabs.add(tabId)
    // Enable observe domains once per attach, not on every call: re-running the
    // three CDP `enable` commands on an already-attached tab is wasted round
    // trips. attachedDebugTabs is cleared on detach/onRemoved, so a re-attach
    // re-enables.
    await enableObserveDomains(tabId)
  }
  await enableFlatSessions(tabId)
}

async function enableObserveDomains(tabId) {
  await enableObserveDomainsForTarget({ tabId })
}

async function enableFlatSessions(tabId) {
  if (flatSessionTabs.has(tabId)) return
  try {
    await chrome.debugger.sendCommand({ tabId }, "Target.setAutoAttach", {
      autoAttach: true,
      waitForDebuggerOnStart: false,
      flatten: true,
      filter: [{ type: "iframe", exclude: false }],
    })
    flatSessionTabs.add(tabId)
  } catch (error) {
    // Older Chrome builds or restricted pages may not expose flat sessions
    // through chrome.debugger. The scripting frame bridge still covers basic
    // cross-origin frame read/click, so keep the main attach usable.
    console.debug("Hope Agent flat-session setup unavailable", error)
  }
}

function handleFlatSessionEvent(tabId, method, params) {
  if (method === "Target.attachedToTarget") {
    const sessionId = params?.sessionId
    if (typeof sessionId !== "string" || sessionId.length === 0) return
    const targetInfo = plainTargetInfo(params?.targetInfo || {})
    const sessions = flatSessionsForTabMap(tabId)
    sessions.set(sessionId, {
      sessionId,
      targetInfo,
      waitingForDebugger: Boolean(params?.waitingForDebugger),
      attachedAt: Date.now(),
    })
    void enableObserveDomainsForTarget({ tabId, sessionId })
    return
  }
  if (method === "Target.detachedFromTarget") {
    const sessionId = params?.sessionId
    if (typeof sessionId === "string") {
      flatSessionsByTab.get(tabId)?.delete(sessionId)
    }
  }
}

async function enableObserveDomainsForTarget(target) {
  await Promise.allSettled([
    chrome.debugger.sendCommand(target, "Runtime.enable", {}),
    chrome.debugger.sendCommand(target, "Network.enable", {}),
    chrome.debugger.sendCommand(target, "Page.enable", {}),
  ])
}

function flatSessionsForTabMap(tabId) {
  let sessions = flatSessionsByTab.get(tabId)
  if (!sessions) {
    sessions = new Map()
    flatSessionsByTab.set(tabId, sessions)
  }
  return sessions
}

async function flatSessionsForTab(tabId) {
  const sessions = Array.from((flatSessionsByTab.get(tabId) || new Map()).values())
  sessions.sort((a, b) => String(a.sessionId).localeCompare(String(b.sessionId)))
  const frameTree = await frameTreeForTab(tabId)
  const sessionsWithMatches = sessions.map((session) => ({
    ...session,
    matchedFrame: matchSessionToFrame(session, frameTree.frames || []),
  }))
  return {
    tabId,
    flatSessionEnabled: flatSessionTabs.has(tabId),
    frameTree,
    sessions: sessionsWithMatches,
  }
}

function flatSessionCount() {
  let count = 0
  for (const sessions of flatSessionsByTab.values()) {
    count += sessions.size
  }
  return count
}

function plainTargetInfo(targetInfo) {
  return {
    targetId: targetInfo.targetId,
    type: targetInfo.type,
    title: targetInfo.title,
    url: targetInfo.url,
    attached: targetInfo.attached,
    canAccessOpener: targetInfo.canAccessOpener,
  }
}

async function frameTreeForTab(tabId) {
  if (!chrome.webNavigation?.getAllFrames) {
    return { tabId, available: false, frames: [] }
  }
  try {
    const frames = await chrome.webNavigation.getAllFrames({ tabId })
    return {
      tabId,
      available: true,
      frames: (frames || []).map(plainFrameInfo).sort((a, b) => {
        if (a.parentFrameId !== b.parentFrameId) return a.parentFrameId - b.parentFrameId
        return a.frameId - b.frameId
      }),
    }
  } catch (error) {
    return {
      tabId,
      available: false,
      error: error instanceof Error ? error.message : String(error),
      frames: [],
    }
  }
}

function plainFrameInfo(frame) {
  return {
    frameId: Number.isInteger(frame.frameId) ? frame.frameId : -1,
    parentFrameId: Number.isInteger(frame.parentFrameId) ? frame.parentFrameId : -1,
    url: frame.url || "",
    documentId: frame.documentId,
    documentLifecycle: frame.documentLifecycle,
    errorOccurred: frame.errorOccurred,
  }
}

function matchSessionToFrame(session, frames) {
  const targetUrl = session?.targetInfo?.url || ""
  if (!targetUrl) return { status: "missing_url" }
  const candidates = frames.filter((frame) => frame.frameId !== 0 && frame.url === targetUrl)
  if (candidates.length === 1) {
    const frame = candidates[0]
    return {
      status: "matched",
      frameId: frame.frameId,
      parentFrameId: frame.parentFrameId,
      documentId: frame.documentId,
      url: frame.url,
    }
  }
  if (candidates.length > 1) {
    return {
      status: "ambiguous",
      candidateFrameIds: candidates.map((frame) => frame.frameId),
      url: targetUrl,
    }
  }
  return { status: "missing", url: targetUrl }
}

function handleDebuggerEvent(source, method, params) {
  const tabId = source?.tabId
  switch (method) {
    case "Runtime.consoleAPICalled":
      pushObserve("console", {
        at: Date.now(),
        tabId,
        level: String(params.type || "log"),
        text: (params.args || [])
          .map((arg) => {
            if (Object.prototype.hasOwnProperty.call(arg, "value")) return JSON.stringify(arg.value)
            return arg.description || arg.type || ""
          })
          .filter(Boolean)
          .join(" "),
      })
      break
    case "Runtime.exceptionThrown": {
      const detail = params.exceptionDetails || {}
      const text = [detail.text, detail.exception?.description].filter(Boolean).join(" - ")
      pushObserve("pageErrors", {
        at: Date.now(),
        tabId,
        level: "exception",
        text,
        url: detail.url,
      })
      break
    }
    case "Network.responseReceived": {
      const response = params.response || {}
      pushObserve("network", {
        at: Date.now(),
        tabId,
        level: String(response.status || "response"),
        text: `${response.url || ""} (${response.mimeType || "unknown"})`,
        url: response.url,
      })
      break
    }
  }
}

function pushObserve(kind, entry) {
  const buf = observeBuffers[kind]
  if (!buf) return
  if (buf.length >= OBSERVE_RING_CAPACITY) {
    buf.shift()
  }
  buf.push(entry)
}

function readObserveBuffer(kind, since, tabId) {
  const normalized = normalizeObserveKind(kind)
  const cutoff = Number.isFinite(since) ? Number(since) : Number.MIN_SAFE_INTEGER
  const wantedTabId = Number(tabId)
  return observeBuffers[normalized].filter((entry) => {
    if (entry.at <= cutoff) return false
    if (!Number.isInteger(wantedTabId)) return true
    return Number(entry.tabId) === wantedTabId
  })
}

function normalizeObserveKind(kind) {
  switch (kind) {
    case "console":
      return "console"
    case "network":
      return "network"
    case "pageErrors":
    case "page_errors":
    case "errors":
      return "pageErrors"
    case "downloads":
    case "download":
      return "downloads"
    default:
      throw new Error(`Unsupported observe kind: ${kind}`)
  }
}

function formatDownloadItem(item) {
  const parts = [`download ${item.id}`]
  if (item.filename) parts.push(item.filename)
  if (item.state) parts.push(`state=${item.state}`)
  if (item.danger && item.danger !== "safe") parts.push(`danger=${item.danger}`)
  if (Number.isFinite(item.bytesReceived) && Number.isFinite(item.totalBytes)) {
    parts.push(`${item.bytesReceived}/${item.totalBytes} bytes`)
  }
  return parts.join(" ")
}

function formatDownloadDelta(delta) {
  const parts = [`download ${delta.id}`]
  for (const key of ["state", "danger", "filename", "error", "paused"]) {
    if (delta[key] && Object.prototype.hasOwnProperty.call(delta[key], "current")) {
      parts.push(`${key}=${delta[key].current}`)
    }
  }
  return parts.join(" ")
}

async function handleDownloadCompleted(downloadId) {
  const items = await chrome.downloads.search({ id: downloadId })
  const item = items[0]
  if (!item) return
  if (!managedDownloads.has(downloadId) && !isHopeControlledDownload(item)) return
  managedDownloads.add(downloadId)
  try {
    const response = await sendNative("extension.download_completed", plainDownloadItem(item), 10_000)
    pushObserve("downloads", {
      at: Date.now(),
      level: "managed",
      text: formatDownloadManaged(response, item),
      url: item.finalUrl || item.url,
    })
  } catch (error) {
    pushObserve("downloads", {
      at: Date.now(),
      level: "policy_error",
      text: `download ${downloadId} landing policy failed: ${errorToMessage(error)}`,
      url: item.finalUrl || item.url,
    })
  } finally {
    managedDownloads.delete(downloadId)
  }
}

function isHopeControlledDownload(item) {
  const tabId = Number(item?.tabId)
  return Number.isInteger(tabId) && (overlayTabs.has(tabId) || attachedDebugTabs.has(tabId))
}

async function pruneManagedDownloads() {
  if (managedDownloads.size === 0 || !chrome.downloads?.search) return
  try {
    for (const id of [...managedDownloads]) {
      const [item] = await chrome.downloads.search({ id })
      // Drop entries that finished or no longer exist — they can leak if no
      // terminal onChanged delta fired (e.g. tab closed mid-download, restart).
      if (!item || item.state === "complete" || item.state === "interrupted") {
        managedDownloads.delete(id)
      }
    }
  } catch {
    // best-effort cleanup
  }
}

function plainDownloadItem(item) {
  return {
    id: item.id,
    tabId: item.tabId,
    url: item.url,
    finalUrl: item.finalUrl,
    filename: item.filename,
    danger: item.danger,
    mime: item.mime,
    totalBytes: item.totalBytes,
    bytesReceived: item.bytesReceived,
    startTime: item.startTime,
    endTime: item.endTime,
  }
}

function formatDownloadManaged(response, item) {
  const path = response?.result?.path || response?.path || item.filename
  return `download ${item.id} completed: ${path}`
}

async function cancelDownload(params) {
  if (!chrome.downloads?.cancel) {
    throw new Error("chrome.downloads.cancel is unavailable")
  }
  const downloadId = requiredDownloadId(params)
  // Ownership check: only cancel downloads Hope is managing (started from a
  // Hope-controlled tab — tracked in managedDownloads). Cancelling an arbitrary
  // id would let the agent abort the user's unrelated downloads.
  if (!managedDownloads.has(downloadId)) {
    throw new Error(
      `download ${downloadId} is not managed by Hope Agent and cannot be cancelled`
    )
  }
  await chrome.downloads.cancel(downloadId)
  pushObserve("downloads", {
    at: Date.now(),
    level: "cancelled",
    text: `download ${downloadId} cancel requested`,
  })
  return { cancelled: true, downloadId }
}

async function extensionStatus() {
  let tabs = []
  try {
    tabs = await chrome.tabs.query({})
  } catch {
    tabs = []
  }
  return {
    extension: "hope-agent-browser-control",
    extensionVersion: chrome.runtime.getManifest().version,
    protocolVersion: PROTOCOL_VERSION,
    nativeHostName: HOST_NAME,
    nativeConnected,
    flatSessionTabs: flatSessionTabs.size,
    flatSessions: flatSessionCount(),
    tabCount: tabs.length,
  }
}

function tabToPlain(tab) {
  return {
    id: tab.id,
    windowId: tab.windowId,
    active: tab.active,
    audible: tab.audible,
    discarded: tab.discarded,
    favIconUrl: tab.favIconUrl,
    groupId: tab.groupId,
    highlighted: tab.highlighted,
    incognito: tab.incognito,
    index: tab.index,
    pinned: tab.pinned,
    status: tab.status,
    title: tab.title,
    url: tab.url,
  }
}

function requiredTabId(params) {
  const tabId = params.tabId ?? params.id
  if (!Number.isInteger(tabId)) {
    throw new Error("Expected integer tabId")
  }
  return tabId
}

function debuggerTarget(params) {
  /** @type {Record<string, unknown>} */
  const target = { tabId: requiredTabId(params) }
  if (typeof params.sessionId === "string" && params.sessionId.length > 0) {
    target.sessionId = params.sessionId
  }
  return target
}

function requiredDownloadId(params) {
  const downloadId = params.downloadId ?? params.id
  if (!Number.isInteger(downloadId) || downloadId < 0) {
    throw new Error("Expected non-negative integer downloadId")
  }
  return downloadId
}

function requiredFrameId(params) {
  const frameId = params.frameId
  if (!Number.isInteger(frameId) || frameId < 0) {
    throw new Error("Expected non-negative integer frameId")
  }
  return frameId
}

function requiredString(params, key) {
  const value = params[key]
  if (typeof value !== "string" || value.length === 0) {
    throw new Error(`Expected non-empty string: ${key}`)
  }
  return value
}

function normalizeMaxElements(value, fallback) {
  if (!Number.isFinite(value)) return fallback
  return Math.max(1, Math.min(Math.floor(Number(value)), 300))
}

function serializeError(error) {
  if (error instanceof Error) {
    return { message: error.message, stack: error.stack }
  }
  return { message: String(error) }
}

function errorToMessage(error) {
  if (error instanceof Error) return error.message
  if (error && typeof error === "object" && "message" in error) return String(error.message)
  return String(error)
}
