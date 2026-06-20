// @ts-check

const statusEl = document.getElementById("status")
const messageEl = document.getElementById("message")
const dotEl = document.getElementById("dot")
const stopCurrentButton = document.getElementById("stop-current")
const stopAllButton = document.getElementById("stop-all")

async function sendMessage(method, params = {}) {
  const response = await chrome.runtime.sendMessage({ method, params })
  if (!response?.ok) {
    throw new Error(response?.error?.message || "Extension command failed")
  }
  return response.result
}

async function activeTabId() {
  const tabs = await chrome.tabs.query({ active: true, currentWindow: true })
  const tabId = tabs[0]?.id
  if (!Number.isInteger(tabId)) {
    throw new Error("No active tab in the current Chrome window")
  }
  return tabId
}

function setBusy(busy) {
  if (stopCurrentButton instanceof HTMLButtonElement) stopCurrentButton.disabled = busy
  if (stopAllButton instanceof HTMLButtonElement) stopAllButton.disabled = busy
}

function setMessage(message) {
  if (messageEl) messageEl.textContent = message
}

async function refreshStatus() {
  try {
    const status = await sendMessage("hope.popup.status")
    if (dotEl) dotEl.classList.toggle("connected", Boolean(status.nativeConnected))
    if (statusEl) {
      const connected = status.nativeConnected ? "Native host connected" : "Native host offline"
      statusEl.textContent = `${connected}. ${status.attachedTabs} attached tab(s), ${status.overlayTabs} overlay tab(s).`
    }
  } catch (error) {
    if (dotEl) dotEl.classList.remove("connected")
    if (statusEl) statusEl.textContent = "Unable to read extension status"
    setMessage(error instanceof Error ? error.message : String(error))
  }
}

async function stopCurrentTab() {
  setBusy(true)
  setMessage("")
  try {
    const tabId = await activeTabId()
    await sendMessage("hope.popup.stopTab", { tabId })
    setMessage(`Stopped control for tab ${tabId}.`)
    await refreshStatus()
  } catch (error) {
    setMessage(error instanceof Error ? error.message : String(error))
  } finally {
    setBusy(false)
  }
}

async function stopAllTabs() {
  setBusy(true)
  setMessage("")
  try {
    const result = await sendMessage("hope.popup.stopAll")
    setMessage(`Stopped ${result.stopped} controlled tab(s).`)
    await refreshStatus()
  } catch (error) {
    setMessage(error instanceof Error ? error.message : String(error))
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

void refreshStatus()
