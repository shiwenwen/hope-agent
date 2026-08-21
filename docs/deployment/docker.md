# Docker 部署

> 简体中文 · [English](docker.en.md)

Hope Agent 提供官方多架构容器镜像，覆盖 `linux/amd64` 与 `linux/arm64`，跟随每次 Release Tag 自动构建并发布到 GitHub Container Registry。

容器化的是 `hope-agent server` 模式 —— 一个内嵌完整 Web GUI 的 HTTP/WebSocket 服务器。浏览器访问容器暴露的端口即可看到与桌面端一致的界面，包含 Onboarding 向导、Provider / MCP / IM Channel 配置面板与全部对话功能。桌面 Tauri GUI 与 ACP stdio 两种模式不适用于容器部署。

## 镜像

```
ghcr.io/shiwenwen/hope-agent:latest
ghcr.io/shiwenwen/hope-agent:v0.2.1
ghcr.io/shiwenwen/hope-agent:0.2
```

预发版本（含 `-rc` / `-beta` 等后缀的 tag）只发不可变 `vX.Y.Z-rcN` tag，不会覆盖 `latest` 与 `X.Y`。

## 快速开始

最简单的启动方式：

```bash
docker run -d \
  --name hope-agent \
  -p 127.0.0.1:8420:8420 \
  -v hope-data:/data \
  ghcr.io/shiwenwen/hope-agent:latest
```

容器跑起来后浏览器打开 <http://127.0.0.1:8420>，按 Onboarding 向导配置 Provider API Key、记忆设置等。所有数据持久化在命名卷 `hope-data` 里，对应容器内 `/data`（即 `HA_DATA_DIR`）。

### 用 docker compose

仓库根目录已提供 [`docker-compose.yml`](../../docker-compose.yml)，复制到部署机器后：

```bash
docker compose up -d
docker compose exec hope-agent hope-agent server token show
docker compose logs -f hope-agent
```

## 配置

### 环境变量

| 变量 | 默认值 | 说明 |
| --- | --- | --- |
| `HA_BIND` | `0.0.0.0:8420` | server 监听地址。容器内必须是 `0.0.0.0`（loopback 会拒绝外部连接）。entrypoint 自动翻译为 `--bind` |
| `HA_API_KEY` | _未设置_ | 可选的外部托管 Owner Root Token。Rust 入口会在运行时初始化前读取并移除，绝不复制到 argv 或工具子进程；浏览器只用它换 HttpOnly 会话，不保存长期 Token |
| `HA_API_KEY_FILE` | _未设置_ | 挂载的 Secret 文件路径，优先于 `HA_API_KEY`。生产环境推荐；文件内容末尾换行会被忽略 |
| `HA_KNOWLEDGE_AGENT_READ_TOKEN` | _未设置_ | Knowledge Agent 只读 token。只能访问 `/api/knowledge/agent/{search,read,expand,sources}`，不能访问 owner 管理 API 或 `compile/propose`；适合给外部 agent 的 HTTP 脚本使用 |
| `HA_CORS_ORIGINS` | _未设置_ | 额外允许的 Web GUI origin，多个值用逗号分隔（如 `https://ui.example`）。仅在前端与 API 跨源部署时设置；同源 UI 与打包桌面 WebView 无需设置，不支持 `*` |
| `HA_SERVER_AUTO_APPROVE_TOOLS` | _未设置_ | 设为 `1` / `true` / `yes` 让 HTTP 入口的每条 chat 都按「自动批准工具」处理 —— 等同于桌面端 IM 渠道账号勾上「auto-approve tools」。**全自动放行**：dangerous-commands / protected-paths / edit-command 审计 / Plan Mode ask / Smart judge **全部跳过**，LLM 触发的任何 `exec` / `write` / `edit` 直接执行无任何拦截。**不要用于不可信租户**。`--dangerously-skip-all-approvals` 是严格超集（还会静默 dispatcher 层审计日志）。无人值守 / CI / pipeline 部署且客户端没接审批 UI 时必开，否则每条 `exec` 都会等满 5 分钟超时 → deny |
| `HA_DATA_DIR` | `/data` | 数据根目录，所有持久化文件（`config.json` / `sessions.db` / `memory.db` / 凭据 / 项目 / 附件等）都在此目录下 |
| `HA_DEPLOYMENT` | `docker` | 给 updater 的部署形态提示。**不要改**，否则 `app_update install` 会尝试在容器内做 binary swap |
| `TZ` | `UTC` | 时区。影响 cron 调度与时间戳格式 |

### 端口与网络

镜像 `EXPOSE 8420`。首次启动若未提供 Token，Docker 会自动生成一枚并以 0600 权限保存在 `/data/credentials/server-auth.json`；用 `docker compose exec hope-agent hope-agent server token show` 查看。`docker-compose.yml` 默认仍只映射宿主机回环地址。

#### LAN / 公网暴露

要让 LAN 或公网访问，保留自动生成的 Token（或配置 `HA_API_KEY_FILE`），再把端口映射改成 `8420:8420`，并前置反代做 TLS 终止。非回环监听缺少 Token 时服务会拒绝启动，不再静默降级。

三种典型部署：

1. **浏览器访问**：首次打开 Auth Gate，粘贴 Root Token 后换取签名 `HttpOnly + SameSite=Strict` Cookie；Root Token 不进 URL、localStorage 或 Referer。之后 HTTP、媒体和 WebSocket 都复用短期会话。
2. **自动化客户端**：继续使用 `Authorization: Bearer <root-token>`。不要使用 query token；通用 `?token=` 已拒绝。
3. **反向代理 / VPN**：公网必须用 HTTPS；VPN 可再收窄网络面，但不会替代内置 Token。反代若另加 OIDC/mTLS 属额外防线。

轮换：设置 → 服务器可在线轮换并只显示新 Token 一次，且会立即让全部旧浏览器会话和 Bearer 客户端失效。CLI 使用 `hope-agent server token rotate` 写入新 Token 后，需重启容器或服务才会激活。若 Token 来自 `HA_API_KEY(_FILE)`，必须在 Secret 源头轮换。

### 数据持久化

容器内 `/data` 是 `HA_DATA_DIR`，包含：

- `config.json` — 全局配置（Provider 列表、记忆设置、温度、failover 策略等）
- `user.json` — 用户偏好
- `sessions.db` / `memory.db` / `logs.db` / `cron.db` — SQLite 数据库
- `credentials/` — Owner Token、Provider API Key、OAuth token、MCP 凭据（**包含敏感信息，文件权限 0600**）
- `agents/` — Agent 定义
- `projects/` — 项目文件
- `attachments/` — 会话附件
- `avatars/` — 头像

**必须挂载为持久卷**，否则容器重启会丢全部历史。`docker-compose.yml` 默认用命名卷 `hope-data`。要用 bind mount：

```yaml
volumes:
  - /srv/hope-agent:/data
```

注意：bind mount 的目录需要 UID 1000 可写（容器内运行用户 `hope` 的 UID）。

## Docker 隔离沙箱

容器化部署只支持 `isolated` 沙箱模式。Hope Agent 会先创建有界临时副本，再通过 Docker Archive API 流式上传到子容器的匿名 `/workspace` volume；命令结束后子容器和匿名 volume 一并删除，修改不会回写真实工作区。这个路径不把容器内的 `/data` 当作宿主机 bind mount，因而同时支持命名卷、bind mount 和 NAS 容器管理器。

`standard` / `workspace` / `trusted` 在容器化部署中会 fail closed。它们需要把实时工作目录 bind mount 到子容器，但 `/data/project` 这类路径属于 Hope Agent 容器命名空间，不能安全地当作 Docker daemon 所见的宿主路径。

启用前需把可信的本机 Docker socket 显式挂入 Hope Agent，并添加 socket 的组 GID：

```bash
stat -c '%A %u:%g %n' /var/run/docker.sock
export DOCKER_GID="$(stat -c '%g' /var/run/docker.sock)"
```

```yaml
services:
  hope-agent:
    volumes:
      - hope-data:/data
      - /var/run/docker.sock:/var/run/docker.sock
    group_add:
      - "${DOCKER_GID}"
```

NAS 图形界面不能展开 `${DOCKER_GID}` 时，先运行 `stat`，再把数字 GID 直接填进附加组。重新创建容器后，沙箱状态会区分 socket 缺失、权限不足、daemon 不可达与客户端配置错误。

> **安全警告**：Docker socket 可控制宿主 Docker daemon，通常等价于宿主机高权限。仅在可信的单租户部署中启用；不要把 socket 改成 `0666`，也不要为了访问 socket 让 Hope Agent 以 root 运行。`isolated` 还要求会话使用项目或显式工作目录；如果工作目录是数据根或其祖先，执行会拒绝，避免把 credentials、配置和数据库复制进沙箱（官方镜像的数据根为 `/data`）。

## 浏览器自动化

镜像内置了 Debian trixie 仓库的 `chromium`（约增加 250 MB 镜像体积）。容器内默认带 `HA_DEPLOYMENT=docker`，所以 Agent 调用浏览器工具时会自动用 headless 模式启动这个 Chromium，并附加容器所需的 sandbox 兼容参数，无需额外配置。

如果你的部署不需要浏览器能力（例如纯 IM 机器人），可以 fork 仓库后从 [`Dockerfile`](../../Dockerfile) 的 runtime 阶段移除 `chromium` 及其依赖（`fonts-liberation` / `libnss3` / `libgbm1` / `libxss1`），重建后镜像更小。

无 `chromium` 包的环境（比如自建的极简镜像）下，agent 仍可以通过 `profile.op=install_runtime` 在运行期下载 manifest 固定版本与 SHA-256 的 Chrome for Testing 兜底，落 `~/.hope-agent/browser/runtime/`。

## Ollama 本地 LLM

镜像本身不打包 Ollama —— Ollama 自己有官方多架构镜像，且模型体积大、GPU 配置复杂，独立 sidecar 更灵活。

启用 Ollama sidecar：

```bash
docker compose --profile with-ollama up -d
```

`docker-compose.yml` 里的 `ollama` 服务：

- 镜像 `ollama/ollama:latest`
- 模型持久化到命名卷 `ollama-models`（容器内 `/root/.ollama`）
- 默认只在 compose 内部网络可达（hope-agent 通过 `http://ollama:11434/v1` 调用）
- GPU passthrough 与 host 端口暴露默认注释掉，按需取消

配置 hope-agent 调用 Ollama：

1. 浏览器进 hope-agent Onboarding / 设置面板
2. 添加 Provider，类型选 **OpenAI Chat**（Ollama 提供 OpenAI 兼容端点）
3. Base URL 填 `http://ollama:11434/v1`
4. API Key 任意填（Ollama 不校验）
5. 模型名按已 pull 的本地模型填，例如 `qwen2.5-coder:7b`

要 pull 模型可以直接 exec 进 Ollama 容器：

```bash
docker compose exec ollama ollama pull qwen2.5-coder:7b
```

### NVIDIA GPU 加速

需要宿主机先装 [nvidia-container-toolkit](https://docs.nvidia.com/datacenter/cloud-native/container-toolkit/install-guide.html)，然后在 `docker-compose.yml` 里把 ollama 服务的 `deploy.resources.reservations.devices` 块取消注释。验证：

```bash
docker compose --profile with-ollama up -d
docker compose exec ollama nvidia-smi
```

## 升级

容器化部署的升级路径与桌面端不同 —— `app_update` 工具检测到 `HA_DEPLOYMENT=docker` 后会跳过 binary swap，引导用户拉新镜像：

```bash
# 用 docker compose
docker compose pull hope-agent
docker compose up -d hope-agent

# 或用 docker run
docker pull ghcr.io/shiwenwen/hope-agent:latest
docker rm -f hope-agent
docker run -d --name hope-agent ... ghcr.io/shiwenwen/hope-agent:latest
```

数据卷会自动复用，配置 / 历史 / 凭据保留。

要锁版本生产环境，推荐固定到具体 tag：`ghcr.io/shiwenwen/hope-agent:v0.2.1`，而非 `latest`。

## 反向代理

生产部署强烈建议前置 Nginx / Caddy / Traefik 做 TLS 终止。Hope Agent 既走 HTTP 又走 WebSocket（`/api/ws/...`），反代必须正确处理 WS upgrade。

Caddy 示例：

```caddyfile
hope.example.com {
    reverse_proxy 127.0.0.1:8420
}
```

Caddy 自动处理 WebSocket upgrade，无需额外配置。

Nginx 示例：

```nginx
server {
    listen 443 ssl http2;
    server_name hope.example.com;

    # TLS 配置略

    location / {
        proxy_pass http://127.0.0.1:8420;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_read_timeout 86400s;
        proxy_send_timeout 86400s;
    }
}
```

## 常见问题

**容器启动后端口拒绝连接？** 容器内 server 必须绑定 `0.0.0.0`。镜像默认 `HA_BIND=0.0.0.0:8420`，不要覆盖为 `127.0.0.1:...`。

**浏览器打开后看到 "Front-end not built" 占位页？** 镜像 build 出错。请检查 `pnpm build` 是否在 `web` 阶段成功（Dockerfile 末尾的 `test -s dist/index.html` 会拦截这种情况）。

**升级后历史消失？** 数据卷没挂对。容器重新创建时确保 `/data` volume 一致。

**ARM Mac (Apple Silicon) 上跑得动吗？** 可以。`linux/arm64` 镜像就是为 Apple Silicon / Raspberry Pi / ARM 云主机准备的，与 amd64 完全等价。

**容器内 `docker exec hope-agent server status` 报 "no server"？** entrypoint 启动时清掉的 `server.pid` 只是为了避免崩溃残留误报。容器内 server 是前台进程（PID 1 是 tini → entrypoint → hope-agent），`server status` 设计用于 systemd / launchd 注册的后台服务，对容器无意义。要查状态用 `docker logs` 或 HEALTHCHECK。

**忘记 Owner Token？** 运行 `docker compose exec hope-agent hope-agent server token show`。若由外部 Secret 管理，这条命令显示当前环境提供的值；请不要把输出贴进日志或工单。

## fork / 自建镜像

如果在 fork 仓库里跑 `.github/workflows/docker.yml`，需要：

- 在 fork 的 GitHub 仓库 Settings → Actions → General → Workflow permissions 启用 `Read and write permissions`（让 `GITHUB_TOKEN` 能 push 到 GHCR）
- 镜像名会自动指向 `ghcr.io/<your-username>/hope-agent` —— workflow 里用 `${{ github.repository_owner }}` 动态拼接

或本地手动构建：

```bash
docker buildx build \
  --platform linux/amd64,linux/arm64 \
  -t hope-agent:dev \
  --load \
  .
```

`--load` 与 multi-platform 同时使用需要 docker engine 23+ 与 containerd image store。
