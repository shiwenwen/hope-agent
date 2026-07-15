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
  // Codex / git worktrees may share node_modules through a symlink. Vite's
  // default node_modules/.vite cache would then be shared as well, allowing
  // another worktree or dev server to invalidate optimized-dependency hashes
  // and cause 504 "Outdated Optimize Dep" responses. Keep the cache local to
  // the current worktree instead.
  cacheDir: path.resolve(__dirname, ".vite-cache"),
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
  build: {
    // Tauri WebView / 现代浏览器都支持 esnext，不必降级转译，省体积与转译开销。
    target: "esnext",
    // 拆 vendor 后单 chunk 远低于此；调高以消除噪音警告。
    chunkSizeWarningLimit: 2000,
    rolldownOptions: {
      output: {
        // 只为「确定 eager」的大块第三方库建独立 vendor chunk（移出主 bundle、命中
        // 长缓存）。刻意不设 catch-all vendor，也不归组 streamdown/remark/rehype——
        // 那些与按需懒加载的 katex/mermaid 共享子依赖，归进 eager 组会把懒加载的重库
        // 拽成 eager（体积反劣化）。katex/mermaid/recharts 等保持各自的 lazy chunk。
        codeSplitting: {
          groups: [
            {
              name: "react-vendor",
              test: /[\\/]node_modules[\\/](react|react-dom|scheduler|use-sync-external-store|react-i18next)[\\/]/,
              priority: 30,
            },
            {
              name: "ui-vendor",
              test: /[\\/]node_modules[\\/](@radix-ui|lucide-react|sonner|class-variance-authority|tailwind-merge|clsx|cmdk|vaul)[\\/]/,
              priority: 25,
            },
            // 不要归组 shiki / @shikijs——它的 76+ 语言是按需 lazy chunk，归进 eager
            // 组会被全量拽成一个 ~9MB 的 eager chunk（体积反劣化）。shiki 的瘦身在
            // Batch 2b 通过 JS 正则引擎 + 语言白名单单独处理。
          ],
        },
      },
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
