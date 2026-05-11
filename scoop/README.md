# Scoop Bucket 模板

此目录是 Scoop bucket（[shiwenwen/scoop-hope-agent](https://github.com/shiwenwen/scoop-hope-agent)）的**单一真相源**。Bucket 仓库由 CI 自动从这里同步——**不要直接在 bucket 仓库手改 manifest**，下次发版会被覆盖。

## 文件

- [`hope-agent.json.tmpl`](hope-agent.json.tmpl) — Scoop manifest 模板。`__PKGVER__` / `__SHA256__` 是 CI 占位符
- [`../.github/workflows/update-scoop-bucket.yml`](../.github/workflows/update-scoop-bucket.yml) — release publish 后自动渲染并推送

## Bucket repo 初始化（一次性）

### 1. 建 bucket repo

```bash
gh repo create shiwenwen/scoop-hope-agent \
  --public \
  --description "🦭 Scoop bucket for Hope Agent · Hope Agent 的 Scoop bucket" \
  --add-readme
```

仓库名约定可以是 `scoop-<name>` / `<name>-bucket` / 任意——Scoop 用户 `scoop bucket add hope-agent https://github.com/shiwenwen/scoop-hope-agent` 自定义别名即可，命名不影响功能，但跟 `homebrew-hope-agent` 同前缀风格更易识别。

### 2. 建 fine-grained PAT 并存到主仓 secret

跟 [`../homebrew/README.md`](../homebrew/README.md) 同样的步骤，区别只在：

- **Token name**: `hope-agent scoop bucket writer`
- **Repository access**: 选 `Only select repositories` → 勾上 `scoop-hope-agent`
- **Repository permissions → Contents**: **`Read and write`**

复制 token 后：

```bash
gh secret set SCOOP_BUCKET_TOKEN --repo shiwenwen/hope-agent
# 回车后粘贴 token + 回车（输入隐藏）
```

## 修改 Manifest 后

直接改 `hope-agent.json.tmpl`，下次发版 CI 会带到 bucket。要立即生效不等下次发版：

```bash
gh workflow run update-scoop-bucket.yml -f tag=vX.Y.Z
```

## 详细发版流程

见 [`../docs/release-process.md`](../docs/release-process.md) §1.8「Scoop bucket 自动同步」。

## 给 Scoop 用户的安装命令

放在 bucket repo 自己的 README 里（不是这里）。本目录仅维护 manifest 模板。
