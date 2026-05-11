# Homebrew Cask 模板

此目录是 Homebrew Cask 的**单一真相源**。Tap 仓库（[shiwenwen/homebrew-hope-agent](https://github.com/shiwenwen/homebrew-hope-agent)）由 CI 自动从这里同步——**不要在 tap repo 里手改 cask 文件**，下一次发版会被覆盖。

## 文件

- [`hope-agent.rb.tmpl`](hope-agent.rb.tmpl) — Cask 模板。`__VERSION__` / `__SHA256__` 是 CI 占位符，由 [`update-homebrew-tap.yml`](../.github/workflows/update-homebrew-tap.yml) 在每次 release publish 时填充并推送到 tap repo 的 `Casks/hope-agent.rb`

## 修改 Cask 后

直接改 `hope-agent.rb.tmpl`，下次发版 CI 会自动把改动带到 tap repo。如果想立即生效（不等下次发版），需要：

1. PR 合到 `main` 或 `release/*`
2. 手动触发 workflow：`gh workflow run update-homebrew-tap.yml -f tag=vX.Y.Z`，CI 会用指定 tag 的 DMG 重新算 sha256、渲染模板、推 tap repo

## Tap repo 初始化（一次性）

新建 [shiwenwen/homebrew-hope-agent](https://github.com/shiwenwen/homebrew-hope-agent)（**仓库名必须是 `homebrew-<tapname>`**，brew 才能识别 `brew tap shiwenwen/hope-agent`）。

初始结构：

```
homebrew-hope-agent/
├── README.md          # 引导用户 brew tap + brew install --cask
└── Casks/
    └── hope-agent.rb  # 由本仓 CI 自动维护，初次可空
```

在主仓 GitHub Settings → Secrets → Actions 加一个 `HOMEBREW_TAP_TOKEN`：

- 类型：Fine-grained PAT
- 仓库范围：仅 `shiwenwen/homebrew-hope-agent`
- 权限：`Contents: Read and write`
- 过期：建议 1 年，到期前提醒续期

详细发版流程见 [`docs/release-process.md` §1.6 Homebrew tap 自动同步](../docs/release-process.md)。
