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
docker compose logs -f hope-agent
```

## 配置

### 环境变量

| 变量 | 默认值 | 说明 |
| --- | --- | --- |
| `HA_BIND` | `0.0.0.0:8420` | server 监听地址。容器内必须是 `0.0.0.0`（loopback 会拒绝外部连接）。entrypoint 自动翻译为 `--bind` |
| `HA_API_KEY` | _未设置_ | HTTP/WS Bearer Token。设了之后从浏览器打开会弹「需要服务器鉴权」对话框，粘贴 token 即可继续；也支持 `https://host:8420/?token=XXX` 一次性传 token —— 前端会自动捕获到 localStorage 并把 URL 清掉。entrypoint 自动翻译为 `--api-key` |
| `HA_SERVER_AUTO_APPROVE_TOOLS` | _未设置_ | 设为 `1` / `true` / `yes` 让 HTTP 入口的每条 chat 都按「自动批准工具」处理 —— 等同于桌面端 IM 渠道账号勾上「auto-approve tools」。**只跳过 chat 入口的工具审批**：dangerous-commands / protected-paths / Plan Mode ask 等 engine 层规则照常执行（要全跳走 `--dangerously-skip-all-approvals`）。无人值守 / CI / pipeline 部署且客户端没接审批 UI 时必开，否则每条 `exec` 都会等满 5 分钟超时 → deny |
| `HA_DATA_DIR` | `/data` | 数据根目录，所有持久化文件（`config.json` / `sessions.db` / `memory.db` / 凭据 / 项目 / 附件等）都在此目录下 |
| `HA_DEPLOYMENT` | `docker` | 给 updater 的部署形态提示。**不要改**，否则 `app_update install` 会尝试在容器内做 binary swap |
| `TZ` | `UTC` | 时区。影响 cron 调度与时间戳格式 |

### 端口与网络

镜像 `EXPOSE 8420`。`docker-compose.yml` 默认把宿主机的 `127.0.0.1:8420` 映射到容器 `8420`，**只允许本机访问** —— 这也是当前推荐的默认部署形态。

#### LAN / 公网暴露

要让 LAN 或公网访问，**先设 `HA_API_KEY`**，再把端口映射改成 `8420:8420`（去掉 `127.0.0.1:` 前缀），最后强烈建议前置反代做 TLS 终止。

三种典型部署：

1. **直接暴露 + 浏览器输 token**：`HA_API_KEY=...` + `0.0.0.0:8420` —— 用户首次访问，前端弹「需要服务器鉴权」对话框，粘 token 即继续；token 存 localStorage，之后访问无感。也可以分享 `https://host:8420/?token=XXX` 一次性预填 token 的链接（前端自动捕获并把 URL 清掉，不进历史 / referer / bookmark）。**风险**：token 在浏览器 localStorage 里，浏览器侧 XSS 会泄露；适合内网 / 小团队。
2. **反向代理注入 Authorization**（推荐生产）：Caddy / Nginx / Traefik 前置 TLS 终止，在 upstream 加 `Authorization: Bearer ${HA_API_KEY}` 头；hope-agent 强制 `HA_API_KEY`，浏览器无需感知 token。用户层访问控制在反代做（client cert / OIDC / basic auth）。
3. **VPN / tailnet 内网**：Tailscale / WireGuard / Zerotier 把容器拉进私网，不开 `HA_API_KEY`，靠网络层隔离。

### 数据持久化

容器内 `/data` 是 `HA_DATA_DIR`，包含：

- `config.json` — 全局配置（Provider 列表、记忆设置、温度、failover 策略等）
- `user.json` — 用户偏好
- `sessions.db` / `memory.db` / `logs.db` / `cron.db` — SQLite 数据库
- `credentials/` — Provider API Key、OAuth token、MCP 凭据（**包含敏感信息**）
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

## 浏览器自动化

镜像内置了 Debian trixie 仓库的 `chromium`（约增加 250 MB 镜像体积）。容器内默认带 `HA_DEPLOYMENT=docker`，所以 Agent 调用浏览器工具时会自动用 headless 模式启动这个 Chromium，并附加容器所需的 sandbox 兼容参数，无需额外配置。

如果你的部署不需要浏览器能力（例如纯 IM 机器人），可以 fork 仓库后从 [`Dockerfile`](../../Dockerfile) 的 runtime 阶段移除 `chromium` 及其依赖（`fonts-liberation` / `libnss3` / `libgbm1` / `libxss1`），重建后镜像更小。

无 `chromium` 包的环境（比如自建的极简镜像）下，agent 仍可以通过 `profile.op=install_runtime` 在运行期下载固定版本的 Chromium snapshot 兜底，落 `~/.hope-agent/browser/runtime/`。

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

**`HA_API_KEY` 没生效？** entrypoint 只在 `CMD` 为 `server start` 时翻译环境变量。如果你覆盖了 CMD（例如 `docker run ... hope-agent server status`），需要手动传 `--api-key` 参数。

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
