# Hope Agent 发版流程

> 分支模型与跨分支红线（`main` / `release/X.Y`、只 cherry-pick 不 merge）见 [AGENTS.md "## 分支与发布"](../AGENTS.md#分支与发布)。本文档是配套实操手册，覆盖在 branch protection 启用下的完整命令流程、避坑要点与速查表。

---

## 0. 心智模型

### 0.1 五个角色

| 角色 | 形态 | 由谁产出 |
| --- | --- | --- |
| 维护分支 `release/X.Y` | git branch | 新 minor 发版后由人工切出，长期存在 |
| Tag `vX.Y.Z` | git tag | 人工在维护分支 HEAD 打，推送后触发 CI |
| GitHub Release | GitHub 资源 | [release.yml](../.github/workflows/release.yml) 自动创建 draft，人工 publish |
| 安装包 | 多平台二进制（DMG / NSIS / DEB / AppImage） | tauri-action 在 CI 中构建并上传到 Release |
| `latest.json` | Tauri updater 清单 | tauri-action 自动生成并上传到 Release，客户端从此拉更新元数据 |

### 0.2 一次发版的全链路

```
PR (含 release notes + CHANGELOG + version bump)
  → 合并到 release/X.Y
  → 在 release/X.Y HEAD 打 tag vX.Y.Z
  → push tag 到 origin
  → release.yml 构建 macOS/Windows/Linux 产物 + latest.json
  → 自动创建 draft GitHub Release，资产齐全
  → 人工审阅 → publish
  → updater endpoint（latest published release/download/latest.json）开始对外服务
  → 已安装客户端"检查更新"拉到新版
```

### 0.3 Tauri updater 拉取链路

客户端配置在 [src-tauri/tauri.conf.json](../src-tauri/tauri.conf.json) `plugins.updater.endpoints`，固定指向 `https://github.com/shiwenwen/hope-agent/releases/latest/download/latest.json`。GitHub 对 `releases/latest` 的解析规则：**只看已 published（非 draft）且非 prerelease 的最新 Release**。因此 draft 状态客户端拉不到，必须人工 publish 后才生效。

### 0.4 版本号单一来源

`package.json` 是版本号唯一真相源，[scripts/sync-version.mjs](../scripts/sync-version.mjs) 把它同步到 [src-tauri/Cargo.toml](../src-tauri/Cargo.toml)、[src-tauri/tauri.conf.json](../src-tauri/tauri.conf.json)、`ha-core`、`ha-server`、`ha-eval` 及 `Cargo.lock`。CI 入口 [scripts/verify-release-version.mjs](../scripts/verify-release-version.mjs) 在 tag 触发后校验所有产品版本来源一致且与 tag 名匹配。

### 0.5 macOS 代码签名

`release.yml` 用一个固定自签名证书签 macOS 包，让授予的系统权限（录屏 / 辅助功能等）跨自动更新不失效（ad-hoc 签名会随 cdhash 变化每次重置授权）。一次性配置（生成证书 + 4 个 GitHub Secrets）见 [macos-self-signing.md](macos-self-signing.md)。Secrets 未配时自动退回 ad-hoc，不影响发布。

---

## 1. patch 发版完整步骤

以从 `release/v0.1` 发 `v0.1.2` 为例。

### 1.1 准备发版 PR

从目标维护分支切发版 PR 分支：

```bash
git checkout release/v0.1 && git pull
git checkout -b release/v0.1.2
```

在 PR 分支上完成三件事（必须同 PR 合并）：

**(a) 写双语 release notes**

新增两份文件：
- [docs/release-notes/v0.1.2.md](release-notes/) — 中文
- [docs/release-notes/v0.1.2.en.md](release-notes/) — 英文

顶部互加 `简体中文 · English` 切换链接（AGENTS.md 强制约定）。文件名必须与 tag 严格对应（带 `v` 前缀），CI 据此填充 `latest.json#notes`。

**链接一律用完整 GitHub URL（tag pin），禁止相对路径**：release notes 内所有跨文件链接必须使用 `https://github.com/shiwenwen/hope-agent/blob/v<X>.<Y>.<Z>/...` 形式的绝对 URL，不要用 `./` 或 `../`。包括中英切换链接、CHANGELOG 锚点、历史 release notes 引用。

原因：[release.yml](../.github/workflows/release.yml) 把 release notes 注入 GitHub Release body 与 `latest.json#notes`；前者 GitHub 会代解析相对路径，后者在桌面应用内的「发现新版」弹窗里渲染时已脱离 GitHub 上下文，相对路径必 broken。tag pin 在 release.yml 触发时（tag push 后）已含本文件，永不漂移；用 `main` 分支引用则需要等 backport 合并才生效，时序上有窗口期。

**(b) 更新 CHANGELOG**

[CHANGELOG.md](../CHANGELOG.md) 顶部新增 `## [0.1.2]` 段，每条 entry 单行 + `(#PR)` 引用，面向用户视角。具体规范见 AGENTS.md "## 文档维护"。

**(c) 同步版本号**

```bash
pnpm version 0.1.2 --no-git-tag-version
```

`--no-git-tag-version` 关键：只改文件、不创建 commit、不打 tag。该命令会触发 `package.json` 中的 `version` script ([sync-version.mjs](../scripts/sync-version.mjs)) 同步 `src-tauri/Cargo.toml` 与 `src-tauri/tauri.conf.json` 三处版本号。

提交 PR：

```bash
git add CHANGELOG.md \
        docs/release-notes/v0.1.2.md docs/release-notes/v0.1.2.en.md \
        package.json src-tauri/Cargo.toml src-tauri/tauri.conf.json
git commit -m "release: v0.1.2"
git push -u origin release/v0.1.2
gh pr create --base release/v0.1 --title "release: v0.1.2" \
  --body "release notes 见 docs/release-notes/v0.1.2.md"
```

### 1.2 合并 PR

走完 8 项 status check（CI 必须全绿），由 reviewer 合并到 `release/v0.1`。merge / squash / rebase 任选，hash 漂移不影响后续 tag。

### 1.3 打 tag 推 tag 触发 CI

PR 合并后，本地：

确认 `release/v0.1` 的 HEAD 就是准备发布的提交后直接打 tag。当前阶段 GitHub Actions 不运行 Capability Evals 或 Live Model Campaign，`release.yml` 也不读取 eval evidence，因此评测不会阻断发布。团队如需发版前回归，可在该 SHA 上通过 App 或 `hope-agent-eval` 本地显式运行；本地结果仅用于人工判断，不会上传或附加到 GitHub Release。

给该 SHA 打 tag：

```bash
git checkout release/v0.1 && git pull
git tag v0.1.2
git push origin v0.1.2
```

`git push origin v0.1.2` 推送的是 tag ref，**不在 branch protection 管辖内**（仓库未配置 tag protection rule），可直推。tag 一旦上 origin，[release.yml](../.github/workflows/release.yml) 立即触发：

1. `release:verify` 校验 `package.json` 版本与 tag 名一致
2. `Extract release notes` 步骤读取 `docs/release-notes/v0.1.2.md`，缺失则 fallback 为 `See CHANGELOG.md for details.`
3. `tauri-action` 在 macOS / Windows / Linux runner 上构建产物
4. 自动创建 draft Release；安装产物与 `latest.json` 按现有发布流程生成

### 1.4 审阅 draft Release 并 publish

GitHub Releases 页找到 `Hope Agent v0.1.2` draft，确认：

- macOS x64 / arm64 DMG 齐全
- Windows NSIS installer 齐全
- Linux AppImage / DEB 齐全
- `latest.json` 在资产列表中
- Release body 显示的是 v0.1.2 release notes 内容（不是 fallback 的 `See CHANGELOG.md for details.`）

确认无误后点 **Publish release**。draft 状态 updater endpoint 抓不到 `latest.json`，**必须 publish 才会推送给已安装客户端**。

### 1.5 backport 到 main

详见 [§3 backport 策略](#3-backport-策略)。

### 1.6 Homebrew tap 自动同步

Publish Release 后 [`.github/workflows/update-homebrew-tap.yml`](../.github/workflows/update-homebrew-tap.yml) 由 `release.published` 事件自动触发：

1. `gh release download` 拉本次 release 的 `Hope.Agent_<version>_aarch64.dmg`
2. `sha256sum` 算 DMG 摘要
3. `sed` 把 [`homebrew/hope-agent.rb.tmpl`](../homebrew/hope-agent.rb.tmpl) 的 `__VERSION__` / `__SHA256__` 占位符替换，渲染为 `Casks/hope-agent.rb`
4. 用 `HOMEBREW_TAP_TOKEN`（fine-grained PAT，仅授 `shiwenwen/homebrew-hope-agent` 的 `Contents: Read and write`）push 到 tap repo

正常路径**不需要任何人工操作**。下列情况需要手动 `gh workflow run update-homebrew-tap.yml -f tag=vX.Y.Z`：

- cask 模板本身改了（如新增 caveats / zap 路径），想立即对已发布版本生效，不等下个 release
- workflow 因为 token 过期 / tap repo 不存在等原因失败过，修复后重跑

**tap repo 初始化（一次性）**：详见 [`homebrew/README.md`](../homebrew/README.md)。仓库名必须是 `homebrew-hope-agent`（Homebrew 约定 `homebrew-<tapname>`），否则 `brew tap shiwenwen/hope-agent` 找不到。

**cask 模板单一真相源在主仓 [`homebrew/hope-agent.rb.tmpl`](../homebrew/hope-agent.rb.tmpl)**。不要在 tap repo 里直接改 `Casks/hope-agent.rb`——下次发版会被 CI 覆盖。

### 1.7 AUR 自动同步

与 §1.6 Homebrew tap 同模式，Release publish 后 [`.github/workflows/update-aur.yml`](../.github/workflows/update-aur.yml) 由 `release.published` 自动触发：

1. `gh release download` 拉本次 release 的 `Hope.Agent_<version>_amd64.deb`
2. `sha256sum` 算 deb 摘要
3. `sed` 渲染 [`aur/hope-agent-bin/PKGBUILD.tmpl`](../aur/hope-agent-bin/PKGBUILD.tmpl) + [`.SRCINFO.tmpl`](../aur/hope-agent-bin/.SRCINFO.tmpl)
4. 用 `AUR_SSH_PRIVATE_KEY`（专用 ed25519 deploy key，公钥已绑到 maintainer 的 AUR 账号）SSH push 到 `ssh://aur@aur.archlinux.org/hope-agent-bin.git`

正常路径**不需要任何人工操作**。下列情况需要手动 `gh workflow run update-aur.yml -f tag=vX.Y.Z`：

- PKGBUILD / .SRCINFO 模板本身改了（如改 deps / 改 pkgdesc），想立即对已发布版本生效
- workflow 因为 SSH key / AUR 账号变化等原因失败过，修复后重跑

**AUR 账号 + SSH key 初始化（一次性）**：详见 [`aur/README.md`](../aur/README.md)。

**模板单一真相源在主仓 [`aur/hope-agent-bin/`](../aur/hope-agent-bin/)**。不要直接 push AUR 仓库——下次发版会被 CI 覆盖。**改 PKGBUILD 字段时必须同步改 .SRCINFO** 字段（两个文件结构是平行的），否则 AUR Web UI 元数据会与 PKGBUILD 不一致。

### 1.8 Scoop bucket 自动同步

与 §1.6 / §1.7 同模式，Release publish 后 [`.github/workflows/update-scoop-bucket.yml`](../.github/workflows/update-scoop-bucket.yml) 由 `release.published` 自动触发：

1. `gh release download` 拉本次 release 的 `Hope.Agent_<version>_x64-setup.exe`
2. `sha256sum` 算 setup.exe 摘要
3. `sed` 渲染 [`scoop/hope-agent.json.tmpl`](../scoop/hope-agent.json.tmpl) → `bucket/hope-agent.json`
4. JSON 语法校验
5. 用 `SCOOP_BUCKET_TOKEN`（fine-grained PAT，仅授 `shiwenwen/scoop-hope-agent` 的 `Contents: Read and write`）push 到 bucket repo

手动重跑：`gh workflow run update-scoop-bucket.yml -f tag=vX.Y.Z`。

**首次配置**：详见 [`scoop/README.md`](../scoop/README.md)。

**Manifest 单一真相源在主仓 [`scoop/hope-agent.json.tmpl`](../scoop/hope-agent.json.tmpl)**。不要在 bucket repo 直接改 `bucket/hope-agent.json`——下次发版会被 CI 覆盖。

> Scoop 默认对 `.exe` URL 用 7zip 解压（不跑 NSIS installer），所以 manifest 不需要 `installer.script`——`hope-agent.exe` 解压出来就是直接可用的单文件 binary。

### 1.9 Linux apt + dnf/yum 软件源自动同步

托管在 **Cloudflare R2**，用户经自定义域名 **`https://repo.hopeagent.ai/`** 访问；用户安装命令见根仓 [`README.md`](../README.md) 「普通用户 → Linux → Debian/Ubuntu」/「Fedora/RHEL/CentOS」段（dnf 与 yum 同 URL 通用，curl 下载 `.repo` 文件方式兼容 dnf4 / dnf5 / yum / zypper）。

> **为何 R2 而非 GitHub Pages**：apt/dnf 索引按同一 base URL 引用包文件，故 `.deb`/`.rpm` 必须托管在该 base 下。GitHub 单文件 100MB 硬上限在包体积破百后会整体拒 push（v0.20.1 arm64 `.deb` 首次越界，v0.21.0 四个包全线越界），旧的 git-commit-to-Pages 方案结构性卡死。R2 无单文件上限、**egress 全免**，是高频下载软件源的自然托管。reprepro / createrepo_c / GPG 构建逻辑不变，只换了发布落点。

Release publish 后 [`.github/workflows/update-linux-repo.yml`](../.github/workflows/update-linux-repo.yml) 由 `release.published` 自动触发：

1. `gh release download` 拉本次 release 的 `Hope.Agent_<v>_amd64.deb` + `Hope.Agent-<v>-1.x86_64.rpm`（arm64/aarch64 若在则一并）
2. `gpg --batch --import` 把 `GPG_SIGNING_KEY` secret 导入临时 `GNUPGHOME`，从 imported key 解出 long fingerprint
3. **`rclone copy r2:$R2_BUCKET → ./bucket`** 把当前已发布的整棵树镜像下来（R2 是单一真相源），令 reprepro `apt/db` 与 createrepo_c 既有 repodata 完整、可增量更新；首次 seed 时为空、从零构建
4. CI 动态渲染 `apt/conf/distributions`（`SignWith:` 填当前 fingerprint，密钥轮换无需改模板），`reprepro -b apt includedeb stable …` 重建 apt index 并签 `InRelease` / `Release.gpg`
5. `createrepo_c --update rpm/stable/<arch>/` 增量更新 yum index，`gpg --detach-sign --armor` 签 `repomd.xml`（产 `repomd.xml.asc`），让 dnf `repo_gpgcheck=1` 能验签
6. 同步 [`linux-repo/rpm/hope-agent.repo`](../linux-repo/rpm/hope-agent.repo) 模板到 `rpm/hope-agent.repo`；从私钥导出公钥到 `pubkey.gpg`（不再手动维护）
7. **`rclone copy ./bucket → r2:$R2_BUCKET`** 非破坏上传（`copy` 非 `sync`）：重生成的索引覆盖、历史包按 checksum 跳过、绝不删除
8. **回抓校验**：经 `https://repo.hopeagent.ai/…` 拉回 `InRelease` / `repomd.xml` / `pubkey.gpg` 断言 live 且格式正确——坏发布（或自定义域名未接好）在 CI 里失败，而非静默把用户留在陈旧源

手动重跑：`gh workflow run update-linux-repo.yml -f tag=vX.Y.Z`（同 tag 重跑幂等——reprepro 先 `remove`，R2 上传非破坏）。

**首次配置（建桶 / 连自定义域名 / R2 API Token / GitHub secrets）+ 密钥轮换流程**：详见 [`linux-repo/README.md`](../linux-repo/README.md)。必备 secret：

- `GPG_SIGNING_KEY` — ed25519 私钥（专用密钥，与 maintainer 个人身份独立）
- `R2_ACCOUNT_ID` / `R2_ACCESS_KEY_ID` / `R2_SECRET_ACCESS_KEY` / `R2_BUCKET` — R2 S3 兼容凭据 + 桶名（旧 `LINUX_REPO_TOKEN` 已退役）

**模板单一真相源在主仓 [`linux-repo/`](../linux-repo/)**。不要直接改 R2 上的 `apt/` / `rpm/`——下次发版 CI 会覆盖。`pubkey.gpg` 现由 CI 从 `GPG_SIGNING_KEY` 导出上传，不再手动 PUT。

> reprepro 的 `apt/db/` 是 incremental state（含 packages 的 sha256 索引）：CI 每次先从 R2 `copy` 下来、改完再 `copy` 回去，故历史版本记录随 R2 持久保存；`apt/conf/distributions` 每次 CI 跑覆盖渲染。R2 上传用 `copy`（非 `sync`）永不删除，一次失败的下载不会误清空。

---

## 2. 新 minor 发版差异

从 `main` 发 `v0.2.0` 为例，与 patch 流程的差异：

### 2.1 PR base 改为 main

§1.1 的 PR base 从 `release/v0.1` 换成 `main`。

### 2.2 tag 在 main HEAD 打

§1.3 的 `git checkout release/v0.1` 改为 `git checkout main`。

### 2.3 发版后切维护分支

tag 推送 + Release publish 完成后，**额外**切一条新维护分支并推送：

```bash
git branch release/v0.2 v0.2.0
git push -u origin release/v0.2
```

CI 触发条件 ([lint.yml](../.github/workflows/lint.yml) / [rust.yml](../.github/workflows/rust.yml) 的 `branches: [main, "release/**"]`) 与 GitHub ruleset `main-branch-protection` 的 `refs/heads/release/**` 通配符自动覆盖新分支，不需要手配。

后续 `v0.2.x` 系列 patch 在 `release/v0.2` 上发，按 §1 流程走。

---

## 3. backport 策略

`release/X.Y` 上的修复**必须 cherry-pick 回 main**，否则下个 minor 发版会丢失这些修复（用户感知为"修过的 bug 又出现"经典回归）。AGENTS.md 红线：**只 cherry-pick 不 merge**。

### 3.1 推荐节奏：按版本批量

每发一版 patch 后立刻批量 cherry-pick 该版本所有 commit 到 main：

```bash
# 列出 v0.1.1 → v0.1.2 之间 release/v0.1 上的所有 commit
git log v0.1.1..v0.1.2 --oneline

# 切 backport 分支并一次性 cherry-pick 整段
git checkout main && git pull
git checkout -b backport/v0.1.2-to-main
git cherry-pick v0.1.1..v0.1.2

# 解冲突（如有）后开 PR
git push -u origin backport/v0.1.2-to-main
gh pr create --base main \
  --title "backport: v0.1.2 fixes to main" \
  --body "cherry-pick 自 release/v0.1, 含 v0.1.1..v0.1.2 全部 commit"
```

N 个 fix 只开一个 backport PR，节奏可控。

### 3.2 跳过等价 commit

某些 commit 在 main 上已经独立存在（如 CI workflow 调整两边各做一次）。`git cherry-pick` 检测到等价改动会冲突或 no-op，遇到时手动跳过：

```bash
git cherry-pick --skip
```

判断方法：commit message 与 diff 跟 main 上某个 commit 完全等价的可跳。

### 3.3 评估"是否需要 backport"

并非所有 patch commit 都需要回种 main：

| 情形 | 处理 |
|---|---|
| main 已重构掉这段代码，bug 只在老分支 | 不 backport |
| main 与 release 都有同样代码 | 必须 backport |
| 只 main 有（新功能引入） | 不在 release 修，只在 main 修 |
| 文档改动只针对老版本 | 不 backport |

仔细评估能砍掉相当比例的 backport 工作量。

### 3.4 cherry-pick 命令速查

```bash
# 单个 commit
git cherry-pick <sha>

# 一段连续 commit（含两端）
git cherry-pick A^..B

# 多个不连续 commit
git cherry-pick sha1 sha2 sha3

# 冲突中
git cherry-pick --continue   # 解完冲突后继续
git cherry-pick --skip       # 跳过当前
git cherry-pick --abort      # 整段放弃

# 自动在 commit message 末尾追加来源链接
git cherry-pick -x <sha>     # 加 (cherry picked from commit <sha>)
```

---

## 4. 关键避坑

| 坑 | 后果 | 规避 |
|---|---|---|
| `pnpm version X.Y.Z` 不带 `--no-git-tag-version` | 本地直接产 commit + tag，但 branch protection 不让推到 `main` / `release/**`，发版卡死 | 一律 `--no-git-tag-version`，version commit 走 PR |
| release notes 文件名错（如 `0.1.2.md` 缺 `v` 前缀，或拼写错误） | `latest.json#notes` 落 fallback 文字 `See CHANGELOG.md for details.`，客户端弹窗看到通用文字 | 文件名严格匹配 tag：`docs/release-notes/v<X>.<Y>.<Z>.md` |
| release notes 用相对路径（`./` `../`） | 注入到 `latest.json#notes` 后桌面应用「发现新版」弹窗里链接全 broken（不在 GitHub 渲染上下文里） | 一律 `https://github.com/shiwenwen/hope-agent/blob/v<X>.<Y>.<Z>/...` 完整 URL（tag pin） |
| 忘记同 PR 合双语 release notes | 中英任一缺失，AGENTS.md 文档约定违例 | 一个发版 PR 必有 4 个文件改动：CHANGELOG + 中文 notes + 英文 notes + 三个 version 文件 |
| draft Release 不 publish | updater endpoint 抓不到，已发布版本对客户端不可见 | §1.4 必须人工 publish |
| 在功能分支 `pnpm version` 带 commit/tag | squash merge 后本地 tag 指向不在 main 上的死 commit，需要重打 | 始终 `--no-git-tag-version`，tag 在 PR 合并后的目标分支 HEAD 上重新打 |
| 跳过 backport 到 main | 下个 minor 丢失所有 patch 修复，回归风险高 | §3.1 每版发完立刻 backport |
| 误用 `git merge release/X.Y → main` | 把维护分支历史污染进 main，AGENTS.md 红线违反 | 只 cherry-pick |
| 新 minor 发版前忘了切 `release/X.Y` 分支 | patch 修复无处落，紧急修复需要回退 main 历史 | §2.3 minor 发布后立即切维护分支 |
| 改 workflow job 名后没同步 ruleset | PR 卡在 status check 等待已不存在的 job | 见 AGENTS.md "## 分支与发布" 末尾 |
| 改 [`release.yml`](../.github/workflows/release.yml) 但没在 PR 阶段验证 | tag push 后跑真实 release 才 fail，删 tag 重打+又一轮 CI 等待。v0.2.0 三次因此返工 | PR CI 会跑 [`scripts/check-release-paths.mjs`](../scripts/check-release-paths.mjs) 静态校验（路径/双架构/必备 platform）；进一步可手动跑 dry-run（见 §4.1） |

### 4.1 修改 release.yml 时的验证流程

任何动 [`release.yml`](../.github/workflows/release.yml) 或 `update-*.yml` 的 PR，按下面两层防护：

**Layer 1 — 自动静态校验**（PR CI 必跑，秒级）

[`.github/workflows/lint.yml`](../.github/workflows/lint.yml) 跑 `node scripts/check-release-paths.mjs`，验证：

- 每个 platform matrix 的 `target_dir=...` 不带 `src-tauri/` 前缀（Hope Agent 是 Cargo workspace，binary 在仓库根 `./target/`）
- Swatinem/rust-cache `workspaces:` 不指向 src-tauri 子目录
- `update-homebrew-tap.yml` / `update-aur.yml` / `update-scoop-bucket.yml` / `update-linux-repo.yml` 引用的 release artifact 文件名模式与 [release.yml](../.github/workflows/release.yml) 实际产出对得上（缺哪个 platform 就 fail / warn）
- matrix 包含 4 个必备 platform（`macos-arm64` / `linux-x64` / `linux-arm64` / `windows-x64`）

本地手动跑：`pnpm check:release-paths`。任何输出 `errors:` 段都会让 PR CI fail。

**Layer 2 — 手动 dry-run**（建议但不强制，~30~40 min）

不确定改动会不会 break 真实 build 时，触发一次 dry-run：

1. 打开 [Actions → Release workflow](https://github.com/shiwenwen/hope-agent/actions/workflows/release.yml)
2. 点 "Run workflow" → branch 选 PR 分支
3. **`tag`** 填一个 sentinel 如 `v0.0.0-dryrun`（不真存在的 tag，verify 步骤自动跳过）
4. **`dry_run`** 勾选为 `true`
5. 跑 → 全 5 平台的 build 矩阵会跑（含 bare-binary path check + signer sign 验证），但跳过：
   - tauri-action 的 draft Release 创建
   - bare-binary upload to release
   - patch-manifest job
6. 产物可在 workflow run 的 `Artifacts` 段下载（`bare-binary-<platform>` artifact）

dry-run **不**改任何 GitHub 状态：不打 tag、不创建 Release、不污染 latest.json。失败重跑无副作用。

**何时必跑 dry-run**：

- 改了 [release.yml](../.github/workflows/release.yml) `Bundle + sign bare binary` step 的 path/case 分支
- 改了 matrix 配置（platform / runner / target / args）
- 改了 [`tauri.conf.json`](../src-tauri/tauri.conf.json) 的 `beforeBuildCommand` / `frontendDist` 或 bundle config
- 引入新的 platform-specific build deps（apt deps / NASM / 等）
- 单纯改 release notes / version bump：不用跑

---

## 5. 命令 / 文件速查

### 5.1 关键脚本

| 命令 | 作用 | 入口 |
|---|---|---|
| `pnpm version X.Y.Z --no-git-tag-version` | 同步版本号到三处文件，不创建 commit/tag | [package.json](../package.json) `scripts.version` → [scripts/sync-version.mjs](../scripts/sync-version.mjs) |
| `pnpm sync:version` | 手动重新同步（一般不用，version 命令自动调） | 同上 |
| `pnpm release:verify -- --tag vX.Y.Z` | 校验三处版本号一致 + 与 tag 名匹配 | [scripts/verify-release-version.mjs](../scripts/verify-release-version.mjs) |
| `pnpm check:release-paths` | 静态校验 release.yml / update-*.yml 路径与 platform 一致性 | [scripts/check-release-paths.mjs](../scripts/check-release-paths.mjs) |

### 5.2 关键文件

| 文件 | 作用 |
|---|---|
| [package.json](../package.json) | 版本号单一真相源 |
| [src-tauri/Cargo.toml](../src-tauri/Cargo.toml) | Rust crate 版本，由 sync-version 同步 |
| [src-tauri/tauri.conf.json](../src-tauri/tauri.conf.json) | Tauri app 版本 + updater endpoint 配置 |
| [.github/workflows/release.yml](../.github/workflows/release.yml) | tag push 触发的发版 workflow，含 release notes 提取逻辑 |
| [docs/release-notes/](release-notes/) | 双语 release notes，文件名 `vX.Y.Z[.en].md` |
| [CHANGELOG.md](../CHANGELOG.md) | 用户视角变更日志，单行 entry + PR 引用 |

### 5.3 常用 git 命令

```bash
# 看 release/v0.1 上 main 没有的 commit（待 backport 候选）
git log origin/main..origin/release/v0.1 --oneline

# 看两版之间的 commit（构造 backport 范围）
git log v0.1.1..v0.1.2 --oneline

# 检查某 commit 是否已在 HEAD
git merge-base --is-ancestor <sha> HEAD && echo "in HEAD" || echo "not in HEAD"

# 查 commit 在哪些分支
git branch --contains <sha>
```

### 5.4 GitHub CLI 速查

```bash
# 看最近的 release
gh release list --limit 5

# 看某 tag 触发的 workflow run
gh run list --workflow release.yml --limit 5

# 查 latest.json 内容（验证发版后 notes 字段填得对不对）
gh release view v0.1.2 --json assets --jq '.assets[] | select(.name=="latest.json") | .url' \
  | xargs curl -sL | jq .
```

---

## 附录：术语对照

- **patch / minor / major**：semver 三段含义，`X.Y.Z` 中的 Z / Y / X
- **维护分支**：`release/X.Y`，长期存在，承载该 minor 的所有 patch
- **PR 临时分支**：每次发版用一次性的 PR 工作分支（命名随意，如 `release/v0.1.2`），合并后删除
- **backport**：把维护分支上的修复 cherry-pick 回 main 的动作
- **draft Release**：CI 创建但未公开的 GitHub Release，updater 不可见
- **updater endpoint**：客户端拉 `latest.json` 的固定 URL，配置在 `tauri.conf.json`
