# AUR 包模板

此目录是 AUR `hope-agent-bin` 包的**单一真相源**。AUR 仓库（`ssh://aur@aur.archlinux.org/hope-agent-bin.git`）由 CI 自动从这里同步——**不要在 AUR 仓库里直接 push**，下次发版会被覆盖。

## 文件

- [`hope-agent-bin/PKGBUILD.tmpl`](hope-agent-bin/PKGBUILD.tmpl) — Arch 包构建脚本。`__PKGVER__` / `__SHA256__` 是 CI 占位符
- [`hope-agent-bin/.SRCINFO.tmpl`](hope-agent-bin/.SRCINFO.tmpl) — AUR Web UI 元数据；字段必须与 PKGBUILD 严格对应，**改 PKGBUILD 时必须同步改 .SRCINFO**
- [`../.github/workflows/update-aur.yml`](../.github/workflows/update-aur.yml) — release publish 后自动渲染推送

## AUR 账号 + SSH key 配置（一次性）

### 1. 注册 AUR 账号

打开 https://aur.archlinux.org/register，填用户名（建议 `shiwenwen` 保持一致）+ 邮箱即可。

### 2. 生成 CI 专用 SSH key

**不要复用你的个人 SSH key**——CI 应该用专门的 deploy key：

```bash
ssh-keygen -t ed25519 -C "hope-agent-aur-ci" -f ~/.ssh/hope-agent-aur-ci -N ""
```

### 3. 把公钥加到 AUR 账号

```bash
cat ~/.ssh/hope-agent-aur-ci.pub
```

复制输出，到 https://aur.archlinux.org/account/<你的用户名>/edit 的 `SSH Public Key` 字段粘贴 → Save。

### 4. 把私钥存到主仓 secret

```bash
gh secret set AUR_SSH_PRIVATE_KEY --repo shiwenwen/hope-agent < ~/.ssh/hope-agent-aur-ci
```

完事后删本地私钥（CI 里已存安全副本，本地不再需要）：

```bash
shred -u ~/.ssh/hope-agent-aur-ci ~/.ssh/hope-agent-aur-ci.pub  # 或 rm -P / rm
```

### 5. 第一次手动 push（可选，CI 也能自动建包）

AUR 的"package 创建"就是第一次 `git push` 到 `ssh://aur@aur.archlinux.org/hope-agent-bin.git` 时 server 端自动建 repo。CI 第一次跑会完成这一步；无需任何手动操作。

## 修改模板后

直接改 `PKGBUILD.tmpl` / `.SRCINFO.tmpl`，下次发版 CI 会带到 AUR。要立即生效不等下次发版：

```bash
gh workflow run update-aur.yml -f tag=vX.Y.Z
```

## 详细发版流程

见 [`../docs/release-process.md`](../docs/release-process.md) §1.7「AUR 自动同步」。
