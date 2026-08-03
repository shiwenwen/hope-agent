# Hope Agent 发版流程

> 分支模型与跨分支红线（`main` / `release/X.Y`、只 cherry-pick 不 merge）见 [AGENTS.md "## 分支与发布"](../AGENTS.md#分支与发布)。本文是实操手册：命令怎么敲、什么不能做。

---

## 0. 心智模型

### 0.1 五个角色

| 角色 | 形态 | 由谁产出 |
| --- | --- | --- |
| 维护分支 `release/X.Y` | git branch | 新 minor 发版后人工切出，长期存在 |
| Tag `vX.Y.Z` | git tag | 人工在目标分支 HEAD 打，推送后触发 CI |
| GitHub Release | GitHub 资源 | [release.yml](../.github/workflows/release.yml) 自动创建 draft，人工 publish |
| 安装包 | DMG / NSIS / DEB / RPM / AppImage | tauri-action 在 CI 构建并上传到 Release |
| `latest.json` | Tauri updater 清单 | tauri-action 生成并上传到 Release，客户端据此拉更新 |

### 0.2 一次发版的全链路

```
PR（release notes + CHANGELOG + version bump）
  → 合并到目标分支
  → 在目标分支 HEAD 打 tag vX.Y.Z 并 push
  → release.yml 构建 4 平台产物 + latest.json
  → 自动创建 draft GitHub Release
  → 人工审阅 → publish
  → 5 条下游渠道 workflow 由 release.published 自动触发（§1.6~§1.10）
  → 已安装客户端「检查更新」拉到新版
```

### 0.3 Tauri updater 拉取链路

客户端按顺序试两个 endpoint，**首个成功者胜**（不比较版本号）：

1. `https://repo.hopeagent.ai/download/latest.json` — R2 镜像，由 §1.10 发布
2. `https://github.com/shiwenwen/hope-agent/releases/latest/download/latest.json` — GitHub 兜底

R2 排第一是**可达性**不是延迟：有一部分用户根本访问不了 `github.com`。

两处配置必须逐项逐序相等：[`tauri.conf.json`](../src-tauri/tauri.conf.json) `plugins.updater.endpoints` ↔ [`manifest.rs`](../crates/ha-core/src/updater/manifest.rs) `UPDATE_MANIFEST_URLS`，由 [`scripts/verify-updater-endpoints.mjs`](../scripts/verify-updater-endpoints.mjs) 在 CI 与 pre-push 强制。

GitHub 的 `releases/latest` 只解析**已 published 且非 prerelease** 的 Release，所以 draft 状态客户端拉不到。

### 0.4 版本号单一来源

`package.json` 是版本号唯一真相源，[scripts/sync-version.mjs](../scripts/sync-version.mjs) 把它同步到 [src-tauri/Cargo.toml](../src-tauri/Cargo.toml)、[src-tauri/tauri.conf.json](../src-tauri/tauri.conf.json)、`ha-base`、`ha-config-schema`、`ha-core`、`ha-acp`、`ha-browser`、`ha-channel`、`ha-cron`、`ha-dash`、`ha-design`、`ha-eval-runtime`、`ha-improve`、`ha-knowledge`、`ha-local-llm`、`ha-mac`、`ha-mcp`、`ha-media`、`ha-pet`、`ha-skills`、`ha-updater`、`ha-vcs`、`ha-weather`、`ha-server`、`ha-browser-host`、`ha-eval` 及 `Cargo.lock`。**禁止手改任何一处**。CI 入口 [scripts/verify-release-version.mjs](../scripts/verify-release-version.mjs) 在 tag 触发后校验所有产品版本来源一致且与 tag 名匹配。

### 0.5 macOS 代码签名

`release.yml` 用固定自签名证书签 macOS 包，让已授予的系统权限（录屏 / 辅助功能）跨自动更新不失效——ad-hoc 签名的 cdhash 每次变，授权每次重置。一次性配置（证书 + 4 个 Secrets）见 [macos-self-signing.md](macos-self-signing.md)；Secrets 未配时自动退回 ad-hoc，不阻塞发布。

---

## 1. patch 发版完整步骤

以从 `release/v0.1` 发 `v0.1.2` 为例。新 minor 的差异见 §2。

### 1.1 准备发版 PR

```bash
git checkout release/v0.1 && git pull
git checkout -b chore/release-v0.1.2
```

> **分支名不能用 `release/v0.1.2`**。`refs/heads/release/**` 是维护分支的受保护命名空间，推该前缀的新分支直接被 ruleset 拒（`8 of 8 required status checks are expected`，等多久都不会好）。一律用 `chore/release-<tag>`。

同一个 PR 里做三件事：

**(a) 双语 release notes**

新增 [`docs/release-notes/v0.1.2.md`](release-notes/) 与 `v0.1.2.en.md`，顶部互加 `简体中文 · English` 切换链接。

- **文件名必须与 tag 严格对应**（带 `v` 前缀）。CI 据此填 `latest.json#notes`，找不到就落 fallback 文字 `See CHANGELOG.md for details.`
- **跨文件链接一律用 `https://github.com/shiwenwen/hope-agent/blob/v0.1.2/...` 绝对 URL，禁用 `./` `../`**。这些链接会进 `latest.json#notes`，在桌面应用的「发现新版」弹窗里渲染时已脱离 GitHub 上下文，相对路径必 broken。tag pin 在 release.yml 触发时已含本文件，永不漂移。
- **不要把 CHANGELOG 条目复制过来**。两者读者不同：CHANGELOG 回答「这个版本改了什么」，release notes 回答「你能拿它做什么」。逐条改写，每句都要过一遍「用户看了能做什么决定」。

  这条反复被违反，所以把判据写死——**下列内容一律不进 release notes**：

  | 禁止出现 | 改写成 |
  | --- | --- |
  | 类型 / 函数 / 文件 / 字段名（`AudioWorklet`、`#memory/...`、`script-src`、`content_hash`） | 用户看到的现象 |
  | 打包与构建概念（「编译进二进制」「sidecar」「产物」「crate」） | 用户要做的动作 |
  | 内部机制（缓存 / 并发 / 上限 / 校验 / 哈希 / 信封 / 门控） | 删掉，或压成一句结果 |
  | 架构与红线（「唯一入口」「fail-closed」「单一真相源」） | 删掉 |
  | 面向开发者的命令与脚本（`pnpm dev:*`、workflow 名） | 删掉，这类进 CHANGELOG 就够了 |

  例外只有一个：**用户自己会去搜、会去敲的东西**可以留（如 `tccutil`、设置项路径、下载域名）。

  安全相关的说法要么写准要么不写——写不准就整句删掉，别用加长解释来补救。

**(b) CHANGELOG**

[CHANGELOG.md](../CHANGELOG.md) 顶部新增 `## [0.1.2]` 段，保留空的 `## [Unreleased]` 在其上。每条 entry 单行 + `(#PR)` 引用，用户视角。规范见 AGENTS.md "## 文档维护"。

**(c) 版本号**

```bash
pnpm version 0.1.2 --no-git-tag-version
```

`--no-git-tag-version` 不能漏：不带它会直接产生 commit + tag，而 branch protection 不让把它们推上去，发版当场卡死。

提交：

```bash
git add CHANGELOG.md \
        docs/release-notes/v0.1.2.md docs/release-notes/v0.1.2.en.md \
        package.json Cargo.lock src-tauri/Cargo.toml src-tauri/tauri.conf.json \
        crates/ha-core/Cargo.toml crates/ha-server/Cargo.toml \
        crates/ha-browser-host/Cargo.toml crates/ha-eval/Cargo.toml
git commit -m "release: v0.1.2"
git push -u origin chore/release-v0.1.2
gh pr create --base release/v0.1 --title "release: v0.1.2" \
  --body "release notes 见 docs/release-notes/v0.1.2.md"
```

推送前自查（都是秒级）：

```bash
RELEASE_TAG=v0.1.2 node scripts/verify-release-version.mjs
node scripts/sync-i18n.mjs --check
node scripts/check-docs-parity.mjs
node scripts/verify-updater-endpoints.mjs
node scripts/check-release-paths.mjs
```

### 1.2 合并 PR

9 项 status check 全绿后合并。merge / squash / rebase 任选，hash 漂移不影响打 tag。

判断"全绿"要**同时校验检查数量 ≥ 9**（Rust scope、cargo fmt、clippy ×3 平台、test ×3 平台、Frontend lint+typecheck）。GitHub 是逐个创建 check run 的，只判"全部非 pending"会在只创建了 1 项时就成立。

CI 全绿仍报 `mergeStateStatus: BLOCKED` 时按这个顺序查：

| 现象 | 原因 | 处理 |
| --- | --- | --- |
| `gh pr checks` 说 no checks reported | PR 与 base 有冲突，GitHub 算不出 `refs/pull/N/merge`，`lint.yml` / `rust.yml` 一项都不会跑 | 看 `mergeStateStatus`，`DIRTY` 即是；rebase 解冲突后 CI 自动开跑 |
| "head branch is not up to date" | base 落后（发版期间 main 常有新 PR 落地） | `git rebase origin/main` 再 force-push，重跑一轮 CI |
| "base branch policy prohibits the merge" | 有未 resolve 的 review 线程（`chatgpt-codex-connector` bot 会自动 review，纯 .md PR 也会） | 先回复说明，再用 GraphQL `resolveReviewThread` |

`gh pr merge --delete-branch` 在 worktree 里必失败（`fatal: 'main' is already used by worktree`），但**合并本身已成功**——去 `gh api -X DELETE repos/.../git/refs/heads/<branch>` 补删，别重试 merge。

### 1.3 打 tag 推 tag 触发 CI

确认目标分支 HEAD 就是要发布的提交，然后：

```bash
git checkout release/v0.1 && git pull
git tag v0.1.2
git push origin v0.1.2
```

tag ref 不在 branch protection 管辖内（仓库未配 tag protection rule），可直推。tag 一上 origin，[release.yml](../.github/workflows/release.yml) 立即：

1. `release:verify` 校验 `package.json` 版本与 tag 名一致
2. 读 `docs/release-notes/v0.1.2.md` 填 Release body 与 `latest.json#notes`
3. tauri-action 在 4 平台 runner 构建产物
4. 创建 draft Release

评测不阻断发版：GitHub Actions 当前不跑 Capability Evals / Live Model Campaign，`release.yml` 也不读 eval evidence。需要发版前回归就在该 SHA 上本地跑 `hope-agent-eval`，结果只供人工判断、不上传。

### 1.4 审阅 draft Release 并 publish

Releases 页找到 `Hope Agent v0.1.2` draft，确认：

- macOS arm64 DMG、Windows NSIS installer、Linux AppImage / DEB / RPM 齐全
- `latest.json` 在资产列表中
- Release body 是本版 release notes，**不是** fallback 的 `See CHANGELOG.md for details.`

确认后点 **Publish release**。draft 状态 updater 抓不到 `latest.json`，不 publish 等于没发。publish 会同时触发 §1.6~§1.10 五条下游渠道。

**publish 之后必须逐条确认五条渠道都 success**，不要默认它们成了：

```bash
TAG=v0.1.2
for wf in update-homebrew-tap update-aur update-scoop-bucket update-linux-repo mirror-release-r2; do
  printf '%-22s ' "$wf"
  gh run list --workflow="$wf.yml" --limit 20 \
    --json displayTitle,conclusion,status,databaseId \
  | jq -r --arg tag "$TAG" '
      [.[] | select(.displayTitle | endswith(" " + $tag))] as $mine
      | ($mine | map(select(.conclusion == "success"))) as $ok
      | if   ($mine | length) == 0 then "本 tag 尚无 run（刚 publish 的话等几秒重查）"
        elif ($ok   | length) >  0 then "OK  run \($ok[0].databaseId)"
        else "未成功  \($mine[0].status)/\($mine[0].conclusion // "-")  run \($mine[0].databaseId)"
        end'
done
```

**必须按 tag 认 run，不能用 `--limit 1`**：那条可能是上一版的 run（本版的还没出现），也可能是某次与本版无关的手动补跑，两种情况都会让清单显示绿而本版根本没跑。五个 workflow 都设了 `run-name: <渠道> <tag>`，所以 `displayTitle` 结尾就是 tag，可以直接匹配。

判定口径是「**本 tag 存在一条 success 的 run**」，不是「最新那条 success」——publish 触发的那条失败、随后手动补跑成功，是正常且完整的结果（v0.27.0 就是这样）。

`mirror-release-r2` 失败最常见，成因与补救见 §1.10（多数情况是 `-f force=true` 重跑）。它失败时 `download/latest.json` 保持旧版本——由于端点是首个成功者胜，**全体客户端会一直被告知「已是最新」**，所以这条不能放着不管。

### 1.5 backport 到 main

见 [§3 backport 策略](#3-backport-策略)。

### 1.6 Homebrew tap 自动同步

`release.published` 触发 [`update-homebrew-tap.yml`](../.github/workflows/update-homebrew-tap.yml)：下载本版 `Hope.Agent_<version>_aarch64.dmg`，算 sha256，渲染 [`homebrew/hope-agent.rb.tmpl`](../homebrew/hope-agent.rb.tmpl) 的 `__VERSION__` / `__SHA256__`，用 `HOMEBREW_TAP_TOKEN` push 到 tap repo。

- 手动重跑：`gh workflow run update-homebrew-tap.yml -f tag=vX.Y.Z`（改了模板想立即对已发布版本生效、或修复 token 后补跑）
- **禁止直接改 tap repo 的 `Casks/hope-agent.rb`**，下次发版会被覆盖。单一真相源是主仓的 `.tmpl`
- tap 仓库名必须是 `homebrew-hope-agent`（Homebrew 约定 `homebrew-<tapname>`），否则 `brew tap shiwenwen/hope-agent` 找不到
- 一次性初始化见 [`homebrew/README.md`](../homebrew/README.md)

### 1.7 AUR 自动同步

`release.published` 触发 [`update-aur.yml`](../.github/workflows/update-aur.yml)：下载本版 `Hope.Agent_<version>_amd64.deb`，算 sha256，渲染 [`PKGBUILD.tmpl`](../aur/hope-agent-bin/PKGBUILD.tmpl) + [`.SRCINFO.tmpl`](../aur/hope-agent-bin/.SRCINFO.tmpl)，用 `AUR_SSH_PRIVATE_KEY` push 到 `ssh://aur@aur.archlinux.org/hope-agent-bin.git`。

- 手动重跑：`gh workflow run update-aur.yml -f tag=vX.Y.Z`
- **禁止直接 push AUR 仓库**，下次发版会被覆盖。单一真相源是 [`aur/hope-agent-bin/`](../aur/hope-agent-bin/)
- **改 PKGBUILD 字段必须同步改 .SRCINFO**（两文件结构平行），否则 AUR Web UI 元数据与 PKGBUILD 不一致
- 一次性初始化见 [`aur/README.md`](../aur/README.md)

### 1.8 Scoop bucket 自动同步

`release.published` 触发 [`update-scoop-bucket.yml`](../.github/workflows/update-scoop-bucket.yml)：下载本版 `Hope.Agent_<version>_x64-setup.exe`，算 sha256，渲染 [`scoop/hope-agent.json.tmpl`](../scoop/hope-agent.json.tmpl)，JSON 语法校验后用 `SCOOP_BUCKET_TOKEN` push 到 bucket repo。

- 手动重跑：`gh workflow run update-scoop-bucket.yml -f tag=vX.Y.Z`
- **禁止直接改 bucket repo 的 `bucket/hope-agent.json`**，下次发版会被覆盖
- manifest 不需要 `installer.script`：Scoop 默认对 `.exe` URL 用 7zip 解压（不跑 NSIS installer），解出来的 `hope-agent.exe` 就是可用单文件
- 一次性初始化见 [`scoop/README.md`](../scoop/README.md)

### 1.9 Linux apt + dnf/yum 软件源自动同步

托管在 Cloudflare R2，用户经 `https://repo.hopeagent.ai/` 访问。安装命令见 [`README.md`](../README.md) 的 Linux 段（dnf 与 yum 同 URL 通用）。

`release.published` 触发 [`update-linux-repo.yml`](../.github/workflows/update-linux-repo.yml)：

1. 下载本版 `.deb` + `.rpm`（amd64/x86_64 与 arm64/aarch64）
2. 导入 `GPG_SIGNING_KEY` 到临时 `GNUPGHOME`，解出 long fingerprint
3. `rclone copy r2:$R2_BUCKET → ./bucket` 把已发布的树整体拉下来（R2 是单一真相源），让 reprepro `apt/db` 与 createrepo_c 既有 repodata 可增量更新
4. 渲染 `apt/conf/distributions`（`SignWith:` 填当前 fingerprint，密钥轮换无需改模板），`reprepro includedeb` 重建索引并签 `InRelease` / `Release.gpg`
5. `createrepo_c --update` 增量更新 yum 索引，`gpg --detach-sign --armor` 签出 `repomd.xml.asc`，供 dnf `repo_gpgcheck=1` 验签
6. 同步 [`hope-agent.repo`](../linux-repo/rpm/hope-agent.repo) 模板，从私钥导出 `pubkey.gpg`
7. `rclone copy ./bucket → r2:$R2_BUCKET` 上传（`copy` 不是 `sync`，绝不删除）
8. 经公开域名回抓 `InRelease` / `repomd.xml` / `pubkey.gpg` 断言 live 且格式正确

- 手动重跑：`gh workflow run update-linux-repo.yml -f tag=vX.Y.Z`（幂等：reprepro 先 `remove`，R2 上传非破坏）
- **禁止直接改 R2 上的 `apt/` / `rpm/`**，下次发版 CI 会覆盖。单一真相源是 [`linux-repo/`](../linux-repo/)
- **`pubkey.gpg` 由 CI 从 `GPG_SIGNING_KEY` 导出**，不要手动 PUT
- **第 3 步的整桶 pull 必须带 `--include "/apt/**" --include "/rpm/**" --include "/pubkey.gpg"`**。同一个桶的 `download/` 前缀（§1.10）每版放约 1.5 GB 且永久保留，不过滤会让这个 pull 无界增长直到超时。新增本 job 独占的顶层路径时同步加进过滤列表
- 必备 secret：`GPG_SIGNING_KEY`（专用 ed25519 私钥，与 maintainer 个人身份独立）、`R2_ACCOUNT_ID` / `R2_ACCESS_KEY_ID` / `R2_SECRET_ACCESS_KEY` / `R2_BUCKET`
- 建桶 / 连自定义域名 / 密钥轮换见 [`linux-repo/README.md`](../linux-repo/README.md)

> **为何是 R2 而非 GitHub Pages**（别改回去）：apt/dnf 索引按同一 base URL 引用包文件，`.deb`/`.rpm` 必须托管在该 base 下。GitHub 单文件 100 MB 硬上限在包体积破百后整体拒 push（v0.20.1 arm64 `.deb` 首次越界，v0.21.0 四个包全线越界），git-commit-to-Pages 方案结构性卡死。R2 无单文件上限、egress 全免。

### 1.10 R2 安装包镜像自动同步

`release.published` 触发 [`mirror-release-r2.yml`](../.github/workflows/mirror-release-r2.yml)，把**全部安装包**镜像到 §1.9 那个桶（同域名）。存在的理由：一部分用户根本访问不了 `github.com`，对他们这不是快慢问题而是能不能装、能不能更新。

桶布局（`apt/` `rpm/` `pubkey.gpg` 属 §1.9，互不重叠）：

```
download/
  v0.26.0/                     ← 不可变，max-age=31536000，永久保留
    Hope.Agent_0.26.0_aarch64.dmg  …（全部资产 + .sig）
    CHANGELOG.md                   ← text/plain，供 notes 链接
    docs/release-notes/v0.26.0.md  docs/release-notes/v0.26.0.en.md
  latest/                      ← 版本无关别名，max-age=300
    Hope.Agent_aarch64.dmg  Hope.Agent_x64-setup.exe  …
  latest.json                  ← 唯一可变 manifest，max-age=60
```

**执行顺序是安全性质，改 workflow 前先读懂**：先传资产与 `latest/` 别名 → 把每个上传对象经**公开域名**回抓、比对 `Content-Length`（只判 200 会放过截断/零长对象），并断言 manifest 里每个 URL 都落在本次前缀内且有本地对应文件 → 全过之后才改写并发布 `latest.json`。所以可变 manifest 绝不会指向不存在的字节；校验失败则 job 失败、`latest.json` 保持原样，R2 上的陈旧 manifest 必然描述一个真实且完整镜像过的版本。修好后重跑：`gh workflow run mirror-release-r2.yml -f tag=vX.Y.Z`（幂等）。

**可变面有 `PROMOTE` 门控**。`download/latest/` 与 `download/latest.json` 全局共享，给非当前稳定版写它们就是一次降级广播——R2 是 endpoint[0] 且首个成功者胜，全体客户端会被告知那个旧版本是最新。两条日常路径会踩到：手动回填旧 tag、发布 prerelease（同样触发 `release.published`）。所以只有该 tag 恰好是 GitHub 认定的 latest release 且非 prerelease 时才推进可变面；其余情况只写自己的不可变前缀然后停下（给 `::notice::`，不算失败）。

**改了 `mirror-release-r2.yml`，本次发版用不上，要下一次才生效**。`release.published` 触发的 run 用的是 **tag 那棵树的 workflow YAML**（`headSha` = 被打 tag 的 commit，`headBranch` = tag 名），哪怕修复早已合进 main。所以：

- 想让修复对**当前这次**发版生效，publish 之后手动补跑一次 `gh workflow run mirror-release-r2.yml --ref main -f tag=vX.Y.Z`——dispatch 用的是 `--ref` 指定分支的 YAML。补跑是幂等的，且此时该 tag 已是 `releases/latest`，`PROMOTE` 会算 true、可变面照常推进。
- 顺序反过来也不行：先合修复再打 tag 才能让 `release.published` 那一轮就带上修复。

**`actions/checkout` 的 `ref:` 是另一回事，必须显式指默认分支**。它决定的是 `scripts/*.mjs` 从哪来，与上面 YAML 的来源无关。`release.published` 下 `github.ref` 就是 `refs/tags/vX.Y.Z`，不写 `ref:` 会静默取那个 tag，两个后果：① 本 workflow 落地**之前**发布的 tag 永远无法镜像（那些树里没有 `scripts/mirror-*.mjs`），整个历史目录不可镜像；② 改写器里修掉的 bug 会冻结在 tag 发布时的样子。文档则相反——必须是该版本实际发布的那一份，所以单独 `git fetch --depth=1` 该 tag 再 `git show <tag>:<path>` 取。

**其余规则**：

- **Cache-Control 用 `--metadata-set` 随 PUT 写入，禁用 `--header-upload`**。`--header-upload` 是 PUT 成功之后的另一次调用，R2 对它返回 501，结果是对象字节正确、Cache-Control 缺失。三档值单一定义在 workflow 的 `CC_IMMUTABLE` / `CC_LATEST` / `CC_MANIFEST`。
- **三条 `rclone copy` 都必须带 `--checksum`，这是正确性不是优化**。少了它 rclone 按 size+modtime 比对，认为已经正确的对象需要刷新 metadata，于是做一次纯 metadata 的服务端 copy——**而那次 copy 落地时没有 Cache-Control**。v0.27.0 实测：force 跑给 28 个对象写上头、校验通过；紧接着一次普通重跑「上传」只花 3 秒（没搬字节），所有不可变对象的 Cache-Control 全部消失。也就是**每次普通重跑都在撤销上一次的修复**。`download/latest/` 那条一直带 `--checksum`，所以它的头在同样的重跑里毫发无损——这个不对称就是判据。
- **头已经丢了要用 `force` 补**：`gh workflow run mirror-release-r2.yml --ref main -f tag=vX.Y.Z -f force=true`。普通重跑修不回来——`--checksum` 看哈希一致就整个跳过，不会重写 metadata。
- **排查 Cache-Control 异常：看指令，不看数字**。别用"是否全体对象都不对"判因，几种成因都只命中一部分对象。
  - **无 Cache-Control**，或裸 `max-age=14400`（无 `public`、无 `must-revalidate`）＝ 头没写上，走上一条的 `force` 重传。两种表现取决于 Cloudflare 那时是否在替源站补默认值，同一个问题。
  - `public, max-age=14400, must-revalidate` ＝ 对象正常，是 Cloudflare Browser Cache TTL 抬高了值。只命中 CF 默认缓存的扩展名（`.dmg` `.exe` `.zip` `.tar.gz`；`.deb` `.rpm` `.AppImage` `.sig` `.json` 走 `DYNAMIC` 不中），且只上调不下调（`download/<tag>/` 的 31536000 不受影响）。要让 `latest/` 的 300s 到达浏览器，Caching → Configuration → Browser Cache TTL 改成 **Respect Existing Headers**。
  - 校验只断言 `immutable` / `must-revalidate` 是否存在，不断言 TTL 数值；TTL 低于设定值才算错误。边缘设置 CI 控制不了，不该卡住发版。
- **rclone 退出码不代表成败，以本 workflow 的回抓校验为准**。R2 会在 PUT 成功之后的某次调用上返回 `501 NotImplemented`，rclone 因此把已经写好的对象报成 `Failed to copy`（实测：报错对象经公开域名 HEAD 全部 200、`Content-Length` 与源文件逐字节一致）。用 `--checksum`，禁用 `--ignore-times`——`--checksum` 发现哈希一致就跳过、整轮自愈；`--ignore-times` 每次强制重传因而每次重报失败，会烧完 10 次重试变成永久失败。
- **rclone 的 stderr 不要用 `tail` 截断**。第一次失败时能定位问题的正是那些被截掉的逐对象错误行。
- **给 rclone 传参的辅助变量不能用 `RCLONE_` 前缀**。rclone 把**任何** `RCLONE_<FLAG>` 环境变量当成同名 flag 的值：`RCLONE_FORCE` 变成 `--force`、`RCLONE_VERSION` 变成 `--version`，第一次调用就以 bool 解析错误退出。`RCLONE_CONFIG_R2_*` 是文档化的 remote 配置写法不受影响；其余一律 `R2_` 前缀。
- **rclone 版本钉住，别用 `apt-get install rclone`**（Ubuntu 24.04 自带 1.60.1）。安装步骤同时断言所需 flag 存在，不支持时在动任何字节之前就报错。
- **临时目录一律落在 `$RUNNER_TEMP` 下，禁用裸相对路径**。`assets/` 曾经是裸路径，于是和仓库自带的 `assets/` 目录合并，把 `alpha-logo.png`、`transparency-logo.png` 当作发布产物镜像了上去。
- **`latest/` 别名不能带 immutable 头**。那些文件名每版复用，长 TTL 会让边缘把旧安装包钉住一年。别名由 [`mirror-latest-aliases.mjs`](../scripts/mirror-latest-aliases.mjs) 按规则剥版本号派生，但 README 实际链接的 8 个名字在 `REQUIRED_ALIASES` 里硬登记——规则失灵时报错，而不是发布一堆 404。两侧对齐由 `check-release-paths.mjs` 在 PR 时守。
- **`notes` 里的 GitHub 链接会被改写成镜像域名**（只改 R2 那份 manifest，仓库源文件不动，§1.1(a) 的绝对 URL 规则不变）。`notes` 是应用内「发现新版」弹窗正文，中英切换与 CHANGELOG 两条链接对目标用户就是死链。链到 `REQUIRED` 之外的文档时 workflow fail-closed。
- **签名原样复制、绝不重算**。信任模型见 [self-update](architecture/self-update.md#manifest-端点链r2-镜像优先github-兜底)。

> 存储成本：一版约 1.5 GB，R2 存储 $0.015/GB·月、egress 免费，按每月 3 版算一年累积约 55 GB（每月 < $1）。刻意全部保留不做清理——现有 R2 发布路径全程只用 `copy` 不用 `sync` 就是为了「绝不删除」，加删除逻辑要单独定义失败语义。

---

## 2. 新 minor 发版差异

从 `main` 发 `v0.2.0`，与 §1 的差异只有三处。

### 2.1 PR base 改为 main

§1.1 的 `gh pr create --base` 从 `release/v0.1` 换成 `main`。分支名仍用 `chore/release-v0.2.0`。

### 2.2 tag 在 main HEAD 打

§1.3 的 `git checkout release/v0.1` 改为 `git checkout main`。

### 2.3 发版后切维护分支

tag 推送 + Release publish 完成后，额外切一条维护分支：

```bash
git branch release/v0.2 v0.2.0
git push -u origin release/v0.2
```

被 ruleset 拒（`push declined due to repository rule violations` + `N of 8 required status checks are in progress`）时不要改配置：`refs/heads/release/**` 同样受必需检查约束，而合并后 main 上那轮 push 触发的 8 项检查此时往往还没跑完。等 `gh api repos/shiwenwen/hope-agent/commits/<sha>/check-runs` 里 8 项全 completed 再重推即可。

> tag 推送不等这些检查，所以 tag 会先成功、切分支后失败，看起来像 tag 打错了。

CI 触发条件（[lint.yml](../.github/workflows/lint.yml) / [rust.yml](../.github/workflows/rust.yml) 的 `branches: [main, "release/**"]`）与 ruleset `main-branch-protection` 的 `refs/heads/release/**` 通配符自动覆盖新分支，不需要手配。

后续 `v0.2.x` 系列 patch 在 `release/v0.2` 上按 §1 发。

---

## 3. backport 策略

`release/X.Y` 上的修复**必须 cherry-pick 回 main**，否则下个 minor 会丢掉它们（用户感知为"修过的 bug 又出现"）。AGENTS.md 红线：**只 cherry-pick 不 merge**。

### 3.1 推荐节奏：按版本批量

每发一版 patch 后立刻批量 cherry-pick 该版本全部 commit，N 个 fix 只开一个 PR：

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
  --body "cherry-pick 自 release/v0.1，含 v0.1.1..v0.1.2 全部 commit"
```

### 3.2 跳过等价 commit

某些 commit 在 main 上已独立存在（如 CI workflow 调整两边各做一次），`git cherry-pick` 会冲突或 no-op。commit message 与 diff 跟 main 上某个 commit 完全等价的，`git cherry-pick --skip` 跳过。

### 3.3 评估是否需要 backport

| 情形 | 处理 |
|---|---|
| main 与 release 都有同样代码 | 必须 backport |
| main 已重构掉这段代码，bug 只在老分支 | 不 backport |
| 只 main 有（新功能引入的 bug） | 不在 release 修，只在 main 修 |
| 文档改动只针对老版本 | 不 backport |

### 3.4 cherry-pick 命令速查

```bash
git cherry-pick <sha>            # 单个
git cherry-pick A^..B            # 一段连续（含两端）
git cherry-pick sha1 sha2 sha3   # 多个不连续
git cherry-pick -x <sha>         # 在 message 末尾追加 (cherry picked from commit <sha>)

git cherry-pick --continue       # 解完冲突后继续
git cherry-pick --skip           # 跳过当前
git cherry-pick --abort          # 整段放弃
```

---

## 4. 关键避坑

| 坑 | 后果 | 规避 |
|---|---|---|
| 发版 PR 分支叫 `release/vX.Y.Z` | 落进受保护命名空间，push 被 ruleset 直接拒，等多久都不会好 | 用 `chore/release-vX.Y.Z`。踩到时 `git branch -m` 改名，commit SHA 不变、pre-push 结果仍有效，可 `HA_SKIP_PREPUSH=1` 重推 |
| `pnpm version X.Y.Z` 不带 `--no-git-tag-version` | 本地直接产 commit + tag，branch protection 不让推，发版卡死 | 一律带 `--no-git-tag-version`，version commit 走 PR |
| release notes 文件名错（缺 `v` 前缀等） | `latest.json#notes` 落 fallback 文字，客户端弹窗看到通用文案 | 文件名严格匹配 tag：`docs/release-notes/v<X>.<Y>.<Z>.md` |
| release notes 用相对路径 | 注入 `latest.json#notes` 后，桌面「发现新版」弹窗里链接全 broken | 一律 `https://github.com/shiwenwen/hope-agent/blob/v<X>.<Y>.<Z>/...`（tag pin） |
| 只写了单语 release notes | 违反 AGENTS.md 双语约定 | 一个发版 PR 至少 4 类改动：CHANGELOG + 中文 notes + 英文 notes + version 文件 |
| draft Release 不 publish | updater 抓不到，5 条下游渠道一条都不触发 | §1.4 必须人工 publish |
| 本机 pre-push 挂在一个与本次改动无关的孤立测试上 | 误以为 main 有回归，白查一轮 | `ha-core` 部分测试读真实 `~/.hope-agent`（例：`permission/global-allowlist.json` 里的「总是允许」会让 permission engine 用例失败）。用 `HA_DATA_DIR=<空目录> git push` 复跑，等价 CI 环境 |
| 跳过 backport 到 main | 下个 minor 丢失全部 patch 修复 | §3.1 每版发完立刻 backport |
| `git merge release/X.Y → main` | 维护分支历史污染进 main，违反 AGENTS.md 红线 | 只 cherry-pick |
| 新 minor 发完忘了切 `release/X.Y` | patch 修复无处落，紧急修复要回退 main 历史 | §2.3 publish 后立即切 |
| 改 workflow job 名后没同步 ruleset | PR 卡在等一个已不存在的 job | 见 AGENTS.md "## 分支与发布" |
| 改 `release.yml` 但没在 PR 阶段验证 | tag push 后跑真实 release 才 fail，删 tag 重打 + 又一轮 CI。v0.2.0 三次因此返工 | §4.1 |

### 4.1 修改 release.yml 时的验证流程

**Layer 1 — 静态校验**（PR CI 必跑，秒级）：[`lint.yml`](../.github/workflows/lint.yml) 跑 `node scripts/check-release-paths.mjs`，验证

- 每个 platform matrix 的 `target_dir=...` 不带 `src-tauri/` 前缀（Hope Agent 是 Cargo workspace，binary 在仓库根 `./target/`）
- Swatinem/rust-cache 的 `workspaces:` 不指向 src-tauri 子目录
- `update-*.yml` 引用的 artifact 文件名模式与 [release.yml](../.github/workflows/release.yml) 实际产出对得上
- matrix 含 4 个必备 platform（`macos-arm64` / `linux-x64` / `linux-arm64` / `windows-x64`）
- README 的 `download/latest/<name>` 链接与 `REQUIRED_ALIASES` 双向一致

本地：`pnpm check:release-paths`。任何 `errors:` 段都会让 PR CI fail。

**Layer 2 — dry-run**（~30~40 min，下列情况必跑）：改了 `Bundle + sign bare binary` step 的 path/case 分支、改了 matrix、改了 [`tauri.conf.json`](../src-tauri/tauri.conf.json) 的 `beforeBuildCommand` / `frontendDist` / bundle config、引入新的 platform-specific build deps。单纯改 release notes 或 version bump 不用跑。

跑法：[Actions → Release workflow](https://github.com/shiwenwen/hope-agent/actions/workflows/release.yml) → Run workflow → branch 选 PR 分支 → `tag` 填一个不存在的 sentinel 如 `v0.0.0-dryrun`（verify 步骤自动跳过）→ `dry_run` 勾 true。全平台 build 矩阵会跑（含 bare-binary path check 与 signer 验证），但跳过 draft Release 创建、bare-binary 上传、patch-manifest job。产物在 run 的 `Artifacts` 段下载。

dry-run 不改任何 GitHub 状态：不打 tag、不建 Release、不碰 latest.json，失败重跑无副作用。

---

## 5. 命令 / 文件速查

### 5.1 关键脚本

| 命令 | 作用 |
|---|---|
| `pnpm version X.Y.Z --no-git-tag-version` | 同步版本号到 8 处文件 + Cargo.lock，不产 commit/tag |
| `pnpm release:verify -- --tag vX.Y.Z` | 校验各处版本号一致且与 tag 匹配 |
| `pnpm check:release-paths` | 静态校验 release.yml / update-*.yml 路径、platform、README 别名一致性 |
| `node scripts/verify-updater-endpoints.mjs` | 校验 updater endpoints 在 tauri.conf.json / manifest.rs / 镜像 PUBLIC_BASE 三处逐项逐序相等 |
| `node scripts/sync-i18n.mjs --check` | 翻译缺失 / 多余 / 插值不一致 / key 顺序漂移 |
| `node scripts/check-docs-parity.mjs` | 用户手册中英章节对齐 |

### 5.2 关键文件

| 文件 | 作用 |
|---|---|
| [package.json](../package.json) | 版本号单一真相源 |
| [src-tauri/tauri.conf.json](../src-tauri/tauri.conf.json) | Tauri app 版本 + updater endpoints + pubkey |
| [crates/ha-core/src/updater/manifest.rs](../crates/ha-core/src/updater/manifest.rs) | headless 侧 updater endpoints，与上一行必须逐项逐序相等 |
| [.github/workflows/release.yml](../.github/workflows/release.yml) | tag push 触发的发版 workflow，含 release notes 提取 |
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
# 最近的 release
gh release list --limit 5

# 某 workflow 的近期 run
gh run list --workflow release.yml --limit 5

# 查 latest.json（验证 notes 字段填得对不对）
gh release view v0.1.2 --json assets --jq '.assets[] | select(.name=="latest.json") | .url' \
  | xargs curl -sL | jq .

# 查镜像那份 latest.json
curl -sL https://repo.hopeagent.ai/download/latest.json | jq '.version, .platforms | keys'

# 某 commit 的检查是否跑完（切维护分支前用）
gh api repos/shiwenwen/hope-agent/commits/<sha>/check-runs --jq '.check_runs[] | "\(.status)\t\(.name)"'
```

---

## 附录：术语对照

- **patch / minor / major**：semver `X.Y.Z` 中的 Z / Y / X
- **维护分支**：`release/X.Y`，长期存在，承载该 minor 的所有 patch。**该前缀受 ruleset 保护**
- **发版 PR 分支**：一次性工作分支，命名 `chore/release-vX.Y.Z`，合并后删除。**不能用 `release/` 前缀**
- **backport**：把维护分支上的修复 cherry-pick 回 main
- **draft Release**：CI 创建但未公开的 GitHub Release，updater 不可见，下游渠道不触发
- **updater endpoint**：客户端拉 `latest.json` 的 URL，两个、按序试、首个成功者胜
