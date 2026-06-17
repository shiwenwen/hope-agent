# macOS 自签名（修复权限授权随更新失效）

## 背景

macOS 把系统权限（**录屏、辅助功能、输入监控**等 TCC 权限）的授权绑定在应用代码签名的 **designated requirement** 上，而不是应用的名字或路径。

发布流水线（`tauri-action`）此前**不配任何签名**，产出的是 **ad-hoc 签名**包：

```
$ codesign -dvvv "/Applications/Hope Agent.app"
Signature=adhoc
designated => cdhash H"f91da6..."      ← 授权被钉死在「这个二进制的精确哈希」上
```

ad-hoc 包的 designated requirement 就是 **cdhash**（二进制哈希）。只要重新构建 / 自动更新，cdhash 就变，macOS 视为「另一个应用」→ 之前授的权对新二进制立即失效。表现为：**系统设置里开关还亮着、应用里却显示「未授权」**。

## 修复

让 CI **每次都用同一个固定证书签名**。证书签名（即使是自签名）的 designated requirement 会变成：

```
designated => identifier "ai.hopeagent.desktop" and certificate leaf = H"<证书哈希>"
```

它只认**证书 + bundle id**（两者跨构建恒定），**不认 cdhash**。于是授权一次后，后续所有更新都满足同一个 requirement，权限不再掉。

无需 Apple Developer ID —— 用一个**固定自签名证书**即可达成「授权持久化」。

> ⚠️ 自签名**消不掉 Gatekeeper 警告**。首次打开仍会提示「未验证的开发者 / 已损坏」，需右键 → 打开，或 `xattr -dr com.apple.quarantine "/Applications/Hope Agent.app"`。要彻底消除得 Developer ID 证书 + 公证（notarization），那是另一回事。

## 一次性配置

1. **在 macOS 上生成固定证书**（只跑一次）：

   ```bash
   bash scripts/macos-selfsign-cert.sh
   ```

   它会打印 4 个值（含私钥材料，**别贴进聊天 / 别提交**）。

2. **加 GitHub 仓库 Secrets**（Settings ▸ Secrets and variables ▸ Actions）：

   | Secret | 值 |
   | --- | --- |
   | `APPLE_CERTIFICATE` | 脚本输出的 base64（`.p12`） |
   | `APPLE_CERTIFICATE_PASSWORD` | 脚本输出的 p12 口令 |
   | `APPLE_SIGNING_IDENTITY` | `Hope Agent Self-Signed` |
   | `KEYCHAIN_PASSWORD` | 脚本输出的临时 keychain 口令 |

   `release.yml` 的 **Set up macOS code signing** 步骤用这 4 个 Secrets 自建专用 keychain、导入证书、`set-key-partition-list` 放行私钥、`sudo security add-trusted-cert` 把它加为可信 code-signing 根，再让 tauri-action 用 `APPLE_SIGNING_IDENTITY` 签。

   > ⚠️ **不能直接用 Tauri 自带的 `APPLE_CERTIFICATE` 导入**：自签名证书默认不被信任，codesign 会报 `no identity found`；Tauri 自带流程不建立信任，所以必须由这个 step 自己建 keychain + 加信任。**Secrets 留空时退回 ad-hoc**（行为不变），先合并不会破坏发布。不配 `APPLE_ID` / `APPLE_TEAM_ID` ⇒ **只签名、不公证**。

3. **先 `workflow_dispatch` 跑 `dry_run=true` 验证**。macOS 签名链路（keychain 信任 + tauri 签名）**只能在 CI runner 上验证**（本地建立 code-signing 信任需要管理员权限、且 tauri 取身份的行为依赖 runner 环境），所以**正式发版前务必先跑一次 dry-run**，确认 `Set up macOS code signing` 的 `find-identity -v -p codesigning` 列出身份、且构建签名不报错。通过后再正式发版。

4. **每台 Mac 重新授权一次**：装上第一个签名版本后，把需要的权限（录屏等）重新授一次 —— 之后所有更新都留得住。

## 注意

- **证书永不能换**。`scripts/macos-selfsign-cert.sh` 不要重复跑去生成新身份 —— 换证书 = 换 designated requirement，已授的权会再失效一次。把 `.p12` / Secrets 当长期凭据保管。
- **首个签名版会让现有授权再失效一次**（designated requirement 从 cdhash 变成证书），重授一次后永久稳定。
- 验证签名是否生效：

  ```bash
  codesign -dvvv "/Applications/Hope Agent.app" 2>&1 | grep -E "Authority|Signature|designated"
  # 期望看到 Authority=Hope Agent Self-Signed、designated => ... certificate leaf ...
  ```
