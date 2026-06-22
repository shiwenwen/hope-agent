// @ts-check

const statusEl = document.getElementById("status")
const messageEl = document.getElementById("message")
const dotEl = document.getElementById("dot")
const stopCurrentButton = document.getElementById("stop-current")
const stopAllButton = document.getElementById("stop-all")

const STATUS_REFRESH_MS = 2500

/**
 * Localized message lookup. Falls back to the key so a missing translation is
 * visible rather than blank (the packaging verifier enforces key parity, so this
 * only ever fires in a broken dev build).
 * @param {string} key
 * @param {string[]=} substitutions
 */
function t(key, substitutions) {
  return chrome.i18n.getMessage(key, substitutions) || key
}

/** Apply localized strings to every element tagged with data-i18n / data-i18n-attr. */
function localizeDom() {
  for (const el of document.querySelectorAll("[data-i18n]")) {
    const key = el.getAttribute("data-i18n")
    const message = key && chrome.i18n.getMessage(key)
    if (message) el.textContent = message
  }
  for (const el of document.querySelectorAll("[data-i18n-attr]")) {
    const spec = el.getAttribute("data-i18n-attr") || ""
    for (const pair of spec.split(",")) {
      const [attr, key] = pair.split(":").map((part) => part.trim())
      const message = attr && key && chrome.i18n.getMessage(key)
      if (message) el.setAttribute(attr, message)
    }
  }
  const uiLocale = chrome.i18n.getMessage("@@ui_locale")
  if (uiLocale) document.documentElement.lang = uiLocale.replace(/_/g, "-")
  if (chrome.i18n.getMessage("@@bidi_dir") === "rtl") document.documentElement.dir = "rtl"
}

async function sendMessage(method, params = {}) {
  const response = await chrome.runtime.sendMessage({ method, params })
  if (!response?.ok) {
    throw new Error(response?.error?.message || t("error_command_failed"))
  }
  return response.result
}

async function activeTabId() {
  const tabs = await chrome.tabs.query({ active: true, currentWindow: true })
  const tabId = tabs[0]?.id
  if (!Number.isInteger(tabId)) {
    throw new Error(t("error_no_active_tab"))
  }
  return tabId
}

function setBusy(busy) {
  if (stopCurrentButton instanceof HTMLButtonElement) stopCurrentButton.disabled = busy
  if (stopAllButton instanceof HTMLButtonElement) stopAllButton.disabled = busy
}

/**
 * @param {string} message
 * @param {boolean=} isError
 */
function setMessage(message, isError = false) {
  if (!messageEl) return
  messageEl.textContent = message
  messageEl.classList.toggle("error", Boolean(message) && isError)
}

async function refreshStatus() {
  try {
    const status = await sendMessage("hope.popup.status")
    if (dotEl) dotEl.classList.toggle("connected", Boolean(status.nativeConnected))
    if (statusEl) {
      statusEl.textContent = status.nativeConnected
        ? t("status_summary", [
            t("status_native_connected"),
            String(status.attachedTabs),
            String(status.overlayTabs),
          ])
        : t("status_native_offline")
    }
  } catch (error) {
    if (dotEl) dotEl.classList.remove("connected")
    if (statusEl) statusEl.textContent = t("error_status_unreadable")
    setMessage(error instanceof Error ? error.message : String(error), true)
  }
}

async function stopCurrentTab() {
  setBusy(true)
  setMessage("")
  try {
    const tabId = await activeTabId()
    await sendMessage("hope.popup.stopTab", { tabId })
    setMessage(t("result_stopped_tab", [String(tabId)]))
    await refreshStatus()
  } catch (error) {
    setMessage(error instanceof Error ? error.message : String(error), true)
  } finally {
    setBusy(false)
  }
}

async function stopAllTabs() {
  setBusy(true)
  setMessage("")
  try {
    const result = await sendMessage("hope.popup.stopAll")
    setMessage(t("result_stopped_count", [String(result.stopped)]))
    await refreshStatus()
  } catch (error) {
    setMessage(error instanceof Error ? error.message : String(error), true)
  } finally {
    setBusy(false)
  }
}

stopCurrentButton?.addEventListener("click", () => {
  void stopCurrentTab()
})

stopAllButton?.addEventListener("click", () => {
  void stopAllTabs()
})

localizeDom()
void refreshStatus()

// Keep the status line fresh while the popup stays open. This also lets the
// cold-start optimistic "connected" flag self-correct within one tick if the
// native host is actually down (onDisconnect resets the flag).
const refreshTimer = setInterval(() => void refreshStatus(), STATUS_REFRESH_MS)
window.addEventListener("unload", () => clearInterval(refreshTimer))
