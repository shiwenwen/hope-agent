import { existsSync, readFileSync } from "node:fs"
import path from "node:path"
import { fileURLToPath } from "node:url"
import { DEFAULT_LOCALE, LOCALE_MESSAGE_FILES } from "./locales.mjs"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const extensionDir = path.resolve(scriptDir, "..")
const manifestPath = path.join(extensionDir, "manifest.json")
const packagePath = path.join(extensionDir, "package.json")
const listingDir = path.join(extensionDir, "store-listing")

const manifest = readJson(manifestPath)
const packageJson = readJson(packagePath)
const zipPath = path.join(
  extensionDir,
  "dist",
  `hope-agent-chrome-extension-${manifest.version}.zip`,
)

const REQUIRED_PACKAGE_FILES = [
  "manifest.json",
  "popup.html",
  "popup.js",
  "service_worker.js",
  "icons/icon16.png",
  "icons/icon32.png",
  "icons/icon48.png",
  "icons/icon128.png",
  ...LOCALE_MESSAGE_FILES,
]
const REQUIRED_PERMISSIONS = [
  "activeTab",
  "alarms",
  "debugger",
  "downloads",
  "nativeMessaging",
  "scripting",
  "tabs",
  "webNavigation",
]
const REQUIRED_HOST_PERMISSIONS = ["http://*/*", "https://*/*"]
const REQUIRED_ICONS = {
  16: "icons/icon16.png",
  32: "icons/icon32.png",
  48: "icons/icon48.png",
  128: "icons/icon128.png",
}

const errors = []

checkManifest()
checkStoreListingDocs()
checkZipPackage()

if (errors.length > 0) {
  console.error("[chrome-extension:verify] failed:")
  for (const error of errors) {
    console.error(`  - ${error}`)
  }
  process.exit(1)
}

console.log(`[chrome-extension:verify] ok ${zipPath}`)

function checkManifest() {
  if (manifest.version !== packageJson.version) {
    errors.push(`manifest version ${manifest.version} does not match package.json ${packageJson.version}`)
  }
  if (manifest.manifest_version !== 3) {
    errors.push("manifest_version must be 3")
  }
  if (!manifest.key) {
    errors.push("unpacked development manifest must keep a stable key")
  }
  if (manifest.minimum_chrome_version !== "116") {
    errors.push("minimum_chrome_version must stay pinned to 116 unless the runtime support matrix is updated")
  }
  if (manifest.default_locale !== DEFAULT_LOCALE) {
    errors.push(`manifest.default_locale must be "${DEFAULT_LOCALE}" (required once name/description use __MSG__ tokens)`)
  }
  if (manifest.background?.service_worker !== "service_worker.js") {
    errors.push("manifest background.service_worker must be service_worker.js")
  }
  expectSameIcons("manifest.icons", manifest.icons)
  expectSameIcons("manifest.action.default_icon", manifest.action?.default_icon)
  expectSameSet("manifest.permissions", manifest.permissions || [], REQUIRED_PERMISSIONS)
  expectSameSet(
    "manifest.host_permissions",
    manifest.host_permissions || [],
    REQUIRED_HOST_PERMISSIONS,
  )
}

function checkStoreListingDocs() {
  const permissionsDoc = readText(path.join(listingDir, "permissions.md"))
  for (const permission of REQUIRED_PERMISSIONS) {
    if (!permissionsDoc.includes(`## \`${permission}\``)) {
      errors.push(`store-listing/permissions.md is missing section for ${permission}`)
    }
  }
  if (!permissionsDoc.includes("## `host_permissions`")) {
    errors.push("store-listing/permissions.md is missing section for host_permissions")
  }

  const reviewNotes = readText(path.join(listingDir, "review-notes.md"))
  for (const permission of ["nativeMessaging", "debugger", "scripting", "downloads", "tabs", "webNavigation"]) {
    if (!reviewNotes.includes(`\`${permission}\``)) {
      errors.push(`store-listing/review-notes.md is missing ${permission}`)
    }
  }

  for (const name of ["en-US.md", "permissions.md", "privacy.md", "review-notes.md", "release-checklist.md"]) {
    const file = path.join(listingDir, name)
    if (!existsSync(file) || readText(file).trim().length === 0) {
      errors.push(`store-listing/${name} is missing or empty`)
    }
  }
}

function checkZipPackage() {
  if (!existsSync(zipPath)) {
    errors.push(`webstore zip does not exist: ${zipPath}; run package:webstore first`)
    return
  }
  const entries = readStoredZipEntries(readFileSync(zipPath))
  expectSameSet("webstore zip entries", [...entries.keys()], REQUIRED_PACKAGE_FILES)
  for (const name of REQUIRED_PACKAGE_FILES) {
    const data = entries.get(name)
    if (!data || data.length === 0) {
      errors.push(`webstore zip entry ${name} is missing or empty`)
    }
  }
  const packagedManifest = parseJsonEntry(entries, "manifest.json")
  if (!packagedManifest) return
  if (packagedManifest.key) {
    errors.push("webstore manifest must not include dev-only key")
  }
  if (packagedManifest.version !== manifest.version) {
    errors.push("webstore manifest version does not match source manifest")
  }
  expectSameSet("webstore manifest.permissions", packagedManifest.permissions || [], REQUIRED_PERMISSIONS)
  expectSameSet(
    "webstore manifest.host_permissions",
    packagedManifest.host_permissions || [],
    REQUIRED_HOST_PERMISSIONS,
  )
  checkLocales(entries)
}

function checkLocales(entries) {
  const defaultFile = `_locales/${DEFAULT_LOCALE}/messages.json`
  const base = parseJsonEntry(entries, defaultFile)
  if (!base || typeof base !== "object") {
    errors.push(`default locale messages missing or invalid: ${defaultFile}`)
    return
  }
  const baseKeys = Object.keys(base).sort()
  if (baseKeys.length === 0) {
    errors.push(`${defaultFile} defines no messages`)
    return
  }
  // Placeholder tokens (e.g. $state$) the translated message MUST preserve so
  // chrome.i18n.getMessage substitution keeps working in every language.
  const basePlaceholders = Object.fromEntries(
    baseKeys.map((key) => [key, Object.keys(base[key]?.placeholders || {})]),
  )

  for (const file of LOCALE_MESSAGE_FILES) {
    const msgs = parseJsonEntry(entries, file)
    if (!msgs || typeof msgs !== "object") {
      errors.push(`locale messages missing or invalid: ${file}`)
      continue
    }
    const keys = Object.keys(msgs).sort()
    if (JSON.stringify(keys) !== JSON.stringify(baseKeys)) {
      const missing = baseKeys.filter((key) => !keys.includes(key))
      const extra = keys.filter((key) => !baseKeys.includes(key))
      errors.push(
        `${file} message keys differ from ${DEFAULT_LOCALE}` +
          (missing.length ? `; missing [${missing.join(", ")}]` : "") +
          (extra.length ? `; extra [${extra.join(", ")}]` : ""),
      )
    }
    for (const key of keys) {
      const message = msgs[key]?.message
      if (typeof message !== "string" || message.trim() === "") {
        errors.push(`${file} key ${key} has an empty or invalid message`)
        continue
      }
      for (const placeholder of basePlaceholders[key] || []) {
        if (!message.toLowerCase().includes(`$${placeholder.toLowerCase()}$`)) {
          errors.push(`${file} key ${key} dropped placeholder $${placeholder}$`)
        }
      }
    }
  }
}

function readStoredZipEntries(buffer) {
  const entries = new Map()
  let offset = 0
  while (offset + 4 <= buffer.length) {
    const sig = buffer.readUInt32LE(offset)
    if (sig === 0x02014b50 || sig === 0x06054b50) break
    if (sig !== 0x04034b50) {
      errors.push(`zip has unexpected signature 0x${sig.toString(16)} at offset ${offset}`)
      break
    }
    const method = buffer.readUInt16LE(offset + 8)
    const compressedSize = buffer.readUInt32LE(offset + 18)
    const uncompressedSize = buffer.readUInt32LE(offset + 22)
    const nameLen = buffer.readUInt16LE(offset + 26)
    const extraLen = buffer.readUInt16LE(offset + 28)
    const nameStart = offset + 30
    const nameEnd = nameStart + nameLen
    const dataStart = nameEnd + extraLen
    const dataEnd = dataStart + compressedSize
    if (method !== 0) {
      errors.push("webstore zip must use stored entries so verifier can inspect package deterministically")
      break
    }
    if (compressedSize !== uncompressedSize) {
      errors.push("webstore zip stored entry size mismatch")
      break
    }
    if (dataEnd > buffer.length) {
      errors.push("webstore zip entry extends beyond file size")
      break
    }
    const name = buffer.subarray(nameStart, nameEnd).toString("utf8")
    entries.set(name, buffer.subarray(dataStart, dataEnd))
    offset = dataEnd
  }
  return entries
}

function parseJsonEntry(entries, name) {
  const data = entries.get(name)
  if (!data) return null
  try {
    return JSON.parse(data.toString("utf8"))
  } catch (error) {
    errors.push(`${name} in webstore zip is invalid JSON: ${error.message}`)
    return null
  }
}

function expectSameIcons(label, actual) {
  if (JSON.stringify(actual || null) !== JSON.stringify(REQUIRED_ICONS)) {
    errors.push(`${label} must declare ${JSON.stringify(REQUIRED_ICONS)}`)
  }
}

function expectSameSet(label, actual, expected) {
  const actualSorted = [...actual].sort()
  const expectedSorted = [...expected].sort()
  if (JSON.stringify(actualSorted) !== JSON.stringify(expectedSorted)) {
    errors.push(`${label} expected [${expectedSorted.join(", ")}], got [${actualSorted.join(", ")}]`)
  }
}

function readJson(file) {
  return JSON.parse(readText(file))
}

function readText(file) {
  return readFileSync(file, "utf8")
}
