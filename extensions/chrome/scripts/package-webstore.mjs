import { mkdirSync, readFileSync, writeFileSync } from "node:fs"
import path from "node:path"
import { fileURLToPath } from "node:url"
import { LOCALE_MESSAGE_FILES } from "./locales.mjs"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const extensionDir = path.resolve(scriptDir, "..")
const manifestPath = path.join(extensionDir, "manifest.json")
const manifest = JSON.parse(readFileSync(manifestPath, "utf8"))
const DOS_TIME = 0
const DOS_DATE = (44 << 9) | (1 << 5) | 1 // 2024-01-01
const CRC32_TABLE = buildCrc32Table()

const packageFiles = [
  {
    name: "manifest.json",
    data: Buffer.from(
      `${JSON.stringify(stripWebStoreOnlyManifestFields(manifest), null, 2)}\n`,
      "utf8",
    ),
  },
  ...[
    "service_worker.js",
    "popup.html",
    "popup.js",
    "icons/icon16.png",
    "icons/icon32.png",
    "icons/icon48.png",
    "icons/icon128.png",
    ...LOCALE_MESSAGE_FILES,
  ].map((name) => ({
    name,
    data: readFileSync(path.join(extensionDir, name)),
  })),
]

const outDir = path.join(extensionDir, "dist")
mkdirSync(outDir, { recursive: true })
const outPath = path.join(outDir, `hope-agent-chrome-extension-${manifest.version}.zip`)
writeFileSync(outPath, buildZip(packageFiles))
console.log(`[chrome-extension:package] wrote ${outPath}`)

function stripWebStoreOnlyManifestFields(input) {
  const out = { ...input }
  delete out.key
  return out
}

function buildZip(files) {
  const localParts = []
  const centralParts = []
  let offset = 0
  for (const file of files) {
    const name = Buffer.from(file.name, "utf8")
    const data = Buffer.from(file.data)
    const crc = crc32(data)
    const localHeader = Buffer.alloc(30)
    localHeader.writeUInt32LE(0x04034b50, 0)
    localHeader.writeUInt16LE(20, 4)
    localHeader.writeUInt16LE(0, 6)
    localHeader.writeUInt16LE(0, 8)
    localHeader.writeUInt16LE(DOS_TIME, 10)
    localHeader.writeUInt16LE(DOS_DATE, 12)
    localHeader.writeUInt32LE(crc, 14)
    localHeader.writeUInt32LE(data.length, 18)
    localHeader.writeUInt32LE(data.length, 22)
    localHeader.writeUInt16LE(name.length, 26)
    localHeader.writeUInt16LE(0, 28)

    localParts.push(localHeader, name, data)

    const centralHeader = Buffer.alloc(46)
    centralHeader.writeUInt32LE(0x02014b50, 0)
    centralHeader.writeUInt16LE(20, 4)
    centralHeader.writeUInt16LE(20, 6)
    centralHeader.writeUInt16LE(0, 8)
    centralHeader.writeUInt16LE(0, 10)
    centralHeader.writeUInt16LE(DOS_TIME, 12)
    centralHeader.writeUInt16LE(DOS_DATE, 14)
    centralHeader.writeUInt32LE(crc, 16)
    centralHeader.writeUInt32LE(data.length, 20)
    centralHeader.writeUInt32LE(data.length, 24)
    centralHeader.writeUInt16LE(name.length, 28)
    centralHeader.writeUInt16LE(0, 30)
    centralHeader.writeUInt16LE(0, 32)
    centralHeader.writeUInt16LE(0, 34)
    centralHeader.writeUInt16LE(0, 36)
    centralHeader.writeUInt32LE(0, 38)
    centralHeader.writeUInt32LE(offset, 42)
    centralParts.push(centralHeader, name)

    offset += localHeader.length + name.length + data.length
  }

  const central = Buffer.concat(centralParts)
  const end = Buffer.alloc(22)
  end.writeUInt32LE(0x06054b50, 0)
  end.writeUInt16LE(0, 4)
  end.writeUInt16LE(0, 6)
  end.writeUInt16LE(files.length, 8)
  end.writeUInt16LE(files.length, 10)
  end.writeUInt32LE(central.length, 12)
  end.writeUInt32LE(offset, 16)
  end.writeUInt16LE(0, 20)

  return Buffer.concat([...localParts, central, end])
}

function crc32(data) {
  let crc = 0xffffffff
  for (const byte of data) {
    crc = CRC32_TABLE[(crc ^ byte) & 0xff] ^ (crc >>> 8)
  }
  return (crc ^ 0xffffffff) >>> 0
}

function buildCrc32Table() {
  const table = new Uint32Array(256)
  for (let i = 0; i < 256; i++) {
    let c = i
    for (let j = 0; j < 8; j++) {
      c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1
    }
    table[i] = c >>> 0
  }
  return table
}
