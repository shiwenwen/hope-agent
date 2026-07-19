# 真实模型 Evidence 签名信任配置

这个目录只保存可公开分发的 Ed25519 公钥注册表；私钥不得进入仓库、Runner 磁盘、
App 配置或评测 artifact。

受保护签名链使用三个彼此独立的控制面：

1. GitHub environment `model-eval-evidence-signing` 保存
   `MODEL_EVAL_EVIDENCE_SIGNING_KEY_PKCS8_B64` secret；
2. 同一 environment 的 `MODEL_EVAL_EVIDENCE_KEY_ID` variable 指向当前 active key；
3. 仓库内 `evidence-keys.json` 保存对应公钥、有效期和状态，并以
   `evidence-keys@<version>` 写入 `evals/live/version-lock.json`。

未配置 `evidence-keys.json` 时，本地真实模型运行、GitHub 普通 evidence 和 Release
preflight 仍可使用，但 App 的“导入受保护基线”保持 fail closed，GitHub `sign` job
显式跳过，不会上传一个无法由发行版验证的 bundle。注册表一旦存在，私钥、key id、签名
或回验任一缺失都会令 `sign` job fail closed。

## 首次配置

1. 在离线或受保护环境生成 Ed25519 PKCS#8 key pair；私钥只写入上述 environment secret。
2. 把 32 字节 raw public key 以标准 Base64 编码，创建 `evidence-keys.json`：

```json
{
  "schemaVersion": "eval-evidence-trust.v1",
  "version": "1.0.0",
  "keys": [
    {
      "id": "model-eval-2026q3",
      "algorithm": "ed25519",
      "publicKey": "<32-byte-public-key-standard-base64>",
      "status": "active",
      "validFrom": "2026-07-18T00:00:00Z"
    }
  ]
}
```

3. 运行 `cargo run -p ha-eval --locked -- model validate`，把输出的 registry digest
   追加到 live version lock，再次校验。
4. 通过配置 PR 合并公钥和 lock；完成 `model-eval-evidence-signing` environment 审批规则后，
   才启用真实签名运行。该 environment 与运行 Provider 的 `model-eval` environment 分离。
5. 下载一次 workflow 产物，用以下命令离线验签：

```bash
cargo run -p ha-eval --locked -- model bundle-verify \
  --bundle eval-evidence-bundle.v1.zip \
  --trust-registry evals/live/trust/evidence-keys.json
```

## 轮换与撤销

- 轮换：先追加新 active key，把旧 key 改为 `retired` 并填写 `validUntil`，提升 registry
  version，追加新 digest；随后再切换 GitHub variable 和 secret。两个版本都必须保留。
- 泄露撤销：把 key 改为 `revoked` 并填写 `revokedAt`，提升 registry version、追加 digest，
立即禁用对应 environment secret。之后的新导入拒绝该 key；此前已导入记录保留原签名
  审计信息，但不得重新晋升或覆盖新的 baseline。
- App 导入时会固化 `key id + SHA-256(raw public key)`；后续刷新必须同时匹配二者。
  禁止在保留 key id 的情况下替换公钥。升级前已有、尚未记录指纹的导入会 fail closed，
  需要用原 signed bundle 重新导入、重新验签后补齐。
- 禁止删除历史 key、改写已有 version-lock digest，或复用 Provider/updater 签名密钥。

Headless Server 不从当前工作目录或可执行文件祖先目录发现注册表。只有确需在 HTTP 只读
Evaluation 查询中刷新签名状态时，管理员才设置 `HA_EVAL_TRUST_REGISTRY_PATH`；它必须是
注册表文件的绝对 canonical path，且路径中不能包含 symlink。未配置或校验失败时查询会把
受保护导入按 key missing 处理。
