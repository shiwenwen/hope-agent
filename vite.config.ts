import { readFileSync } from "node:fs"
import { defineConfig } from "vitest/config"
import react from "@vitejs/plugin-react"
import tailwindcss from "@tailwindcss/vite"
import Icons from "unplugin-icons/vite"
import path from "path"

const packageJson = JSON.parse(
  readFileSync(new URL("./package.json", import.meta.url), "utf8"),
) as {
  version: string
}

// https://vite.dev/config/
export default defineConfig({
  define: {
    __APP_VERSION__: JSON.stringify(packageJson.version),
  },
  plugins: [
    react(),
    tailwindcss(),
    // Build-time inline of the curated vscode-icons file-type icons used by
    // `FileTypeIcon` (offline, tree-shaken — only imported icons are bundled).
    Icons({ compiler: "jsx", jsx: "react", autoInstall: false }),
  ],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  server: {
    port: 1420,
    strictPort: true,
  },
  test: {
    // Default to node — pure-logic tests don't need DOM. Component tests
    // opt in per-file with `// @vitest-environment jsdom` at the top.
    environment: "node",
    globals: false,
    setupFiles: ["./vitest.setup.ts"],
    include: ["src/**/*.{test,spec}.{ts,tsx}"],
  },
})
