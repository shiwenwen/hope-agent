# Docker Deployment

> [简体中文](docker.md) · English

Hope Agent ships official multi-arch container images covering `linux/amd64` and `linux/arm64`, built and pushed automatically to GitHub Container Registry on every release tag.

What's containerized is the `hope-agent server` mode — an HTTP/WebSocket server that embeds the full Web GUI. Visit the exposed port in a browser and you get the same interface as the desktop app: onboarding wizard, Provider / MCP / IM Channel configuration, full chat. The Tauri desktop GUI and the ACP stdio mode are not container-suitable.

## Images

```
ghcr.io/shiwenwen/hope-agent:latest
ghcr.io/shiwenwen/hope-agent:v0.2.1
ghcr.io/shiwenwen/hope-agent:0.2
```

Pre-release tags (anything with `-rc` / `-beta` suffix) only publish the immutable `vX.Y.Z-rcN` tag. They never overwrite `latest` or the `X.Y` floating tag.

## Quick start

The simplest way to get going:

```bash
docker run -d \
  --name hope-agent \
  -p 127.0.0.1:8420:8420 \
  -v hope-data:/data \
  ghcr.io/shiwenwen/hope-agent:latest
```

Once the container is up, open <http://127.0.0.1:8420> in a browser and follow the onboarding wizard to configure provider API keys and memory settings. All state lives in the named volume `hope-data`, which mounts to `/data` (`HA_DATA_DIR`) inside the container.

### With docker compose

A reference [`docker-compose.yml`](../../docker-compose.yml) lives at the repo root:

```bash
docker compose up -d
docker compose exec hope-agent hope-agent server token show
docker compose logs -f hope-agent
```

## Configuration

### Environment variables

| Variable | Default | Purpose |
| --- | --- | --- |
| `HA_BIND` | `0.0.0.0:8420` | Server listen address. Must be `0.0.0.0` inside a container — loopback rejects external connections. Translated to `--bind` by the entrypoint |
| `HA_API_KEY` | _unset_ | Optional externally managed Owner Root Token. Rust consumes and removes it before runtime initialization; it is never copied to argv or tool subprocesses. Browsers exchange it for an HttpOnly session and do not retain the root token |
| `HA_API_KEY_FILE` | _unset_ | Mounted secret-file path, preferred over `HA_API_KEY` for production. A trailing newline is ignored |
| `HA_KNOWLEDGE_AGENT_READ_TOKEN` | _unset_ | Knowledge Agent read-only token. It can only access `/api/knowledge/agent/{search,read,expand,sources}`, not owner admin APIs or `compile/propose`; useful for external-agent HTTP scripts |
| `HA_DATA_DIR` | `/data` | Data root. All persistent state (`config.json` / `sessions.db` / `memory.db` / credentials / projects / attachments) lives here |
| `HA_DEPLOYMENT` | `docker` | Hint to the self-updater. **Do not change** — without it `app_update install` would attempt an in-container binary swap |
| `TZ` | `UTC` | Timezone. Affects cron scheduling and timestamp formatting |

### Ports and networking

The image `EXPOSE`s `8420`. Without an supplied token, Docker generates one on first boot and stores it at `/data/credentials/server-auth.json` with mode 0600. Retrieve it with `docker compose exec hope-agent hope-agent server token show`. Compose still binds the host loopback address by default.

#### LAN / public exposure

For LAN or public access, keep the generated token (or configure `HA_API_KEY_FILE`), change the mapping to `8420:8420`, and terminate TLS at a reverse proxy. A non-loopback server without a token now refuses to start instead of silently downgrading.

Three typical patterns:

1. **Browser access**: the first visit shows the Auth Gate. The Root Token is exchanged for a signed `HttpOnly + SameSite=Strict` cookie and never enters URLs, localStorage, or the Referer. HTTP, media, and WebSocket requests reuse that short-lived session.
2. **Automation clients**: continue to send `Authorization: Bearer <root-token>`. Generic `?token=` authentication is rejected.
3. **Reverse proxy / VPN**: use HTTPS for public exposure. A VPN narrows network reach but does not replace the built-in token; OIDC or mTLS at the proxy is an additional layer.

Rotate online in Settings → Server to invalidate every prior browser session and Bearer client immediately. The CLI command `hope-agent server token rotate` stores a new token; restart the container or service to activate it. For `HA_API_KEY(_FILE)`, rotate the external secret at its source.

### Persistent data

The container's `/data` (`HA_DATA_DIR`) holds:

- `config.json` — global config (provider list, memory settings, temperature, failover policy)
- `user.json` — user preferences
- `sessions.db` / `memory.db` / `logs.db` / `cron.db` — SQLite databases
- `credentials/` — owner token, provider API keys, OAuth tokens, MCP credentials (**sensitive; files use mode 0600**)
- `agents/` — agent definitions
- `projects/` — project-scoped files
- `attachments/` — chat attachments
- `avatars/` — avatar images

**Always mount this as a persistent volume**, otherwise a container restart drops everything. `docker-compose.yml` uses a named volume `hope-data` by default. For a bind mount:

```yaml
volumes:
  - /srv/hope-agent:/data
```

The directory must be writable by UID 1000 (the in-container `hope` user).

## Browser automation

The image bundles Debian trixie's `chromium` package (adds ~250 MB to the image). The container sets `HA_DEPLOYMENT=docker`, so browser tool calls automatically start this Chromium in headless mode with the container-compatible sandbox flag — no extra configuration required.

If your deployment doesn't need browser automation (e.g. a pure IM bot), fork the repo and remove `chromium` plus its runtime libs (`fonts-liberation` / `libnss3` / `libgbm1` / `libxss1`) from the [`Dockerfile`](../../Dockerfile)'s runtime stage to slim the image down.

Even without a `chromium` package, the agent can fall back to `profile.op=install_runtime`, which downloads a pinned Chromium snapshot to `~/.hope-agent/browser/runtime/` at first use.

## Ollama for local LLMs

The image does not bundle Ollama — Ollama has its own well-maintained multi-arch image, models are large, and GPU passthrough adds complexity. Keeping it as a separate sidecar gives users full control.

Enable the Ollama sidecar:

```bash
docker compose --profile with-ollama up -d
```

What the `ollama` service in `docker-compose.yml` does:

- Pulls `ollama/ollama:latest`
- Persists models in the named volume `ollama-models` (maps to `/root/.ollama` inside)
- By default only reachable from inside the compose network — Hope Agent talks to it over `http://ollama:11434/v1`
- GPU passthrough and host port exposure are commented out by default; uncomment as needed

Wire Hope Agent to Ollama:

1. In the browser, open Hope Agent's onboarding / settings panel
2. Add a new Provider, type **OpenAI Chat** (Ollama exposes an OpenAI-compatible API)
3. Set Base URL to `http://ollama:11434/v1`
4. API Key can be anything (Ollama doesn't validate it)
5. Model name should match a model you've pulled, e.g. `qwen2.5-coder:7b`

Pull models from inside the Ollama container:

```bash
docker compose exec ollama ollama pull qwen2.5-coder:7b
```

### NVIDIA GPU acceleration

Install [nvidia-container-toolkit](https://docs.nvidia.com/datacenter/cloud-native/container-toolkit/install-guide.html) on the host first, then uncomment the `deploy.resources.reservations.devices` block in `docker-compose.yml`. Verify:

```bash
docker compose --profile with-ollama up -d
docker compose exec ollama nvidia-smi
```

## Upgrades

The container upgrade path is different from the desktop bundle — the `app_update` tool detects `HA_DEPLOYMENT=docker` and routes the user to image pull instead of a binary swap:

```bash
# Using docker compose
docker compose pull hope-agent
docker compose up -d hope-agent

# Or using docker run
docker pull ghcr.io/shiwenwen/hope-agent:latest
docker rm -f hope-agent
docker run -d --name hope-agent ... ghcr.io/shiwenwen/hope-agent:latest
```

The data volume is preserved across image swaps; config, history, and credentials survive.

For production, pin to a concrete tag like `ghcr.io/shiwenwen/hope-agent:v0.2.1` rather than relying on `latest`.

## Reverse proxy

For production deployments, put Nginx / Caddy / Traefik in front for TLS termination. Hope Agent serves both HTTP and WebSocket (`/api/ws/...`), so the proxy must handle WS upgrade correctly.

Caddy example:

```caddyfile
hope.example.com {
    reverse_proxy 127.0.0.1:8420
}
```

Caddy handles WebSocket upgrades automatically; no extra config needed.

Nginx example:

```nginx
server {
    listen 443 ssl http2;
    server_name hope.example.com;

    # TLS config omitted

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

## FAQ

**Port refuses connections after starting?** The server inside the container must bind `0.0.0.0`. The image sets `HA_BIND=0.0.0.0:8420` by default — do not override to `127.0.0.1:...`.

**Browser shows a "Front-end not built" placeholder page?** The image build failed silently. Check that `pnpm build` succeeded in the `web` stage (the Dockerfile has a `test -s dist/index.html` assertion that should catch this).

**History disappeared after upgrade?** The data volume wasn't mounted. When recreating the container, make sure `/data` points at the same volume.

**Does it run on Apple Silicon / Raspberry Pi?** Yes. The `linux/arm64` image is built exactly for Apple Silicon, Raspberry Pi 4/5, and ARM cloud VMs; functionally identical to amd64.

**`docker exec hope-agent server status` reports "no server"?** The entrypoint clears `server.pid` on startup to avoid stale-PID misreporting. The server inside the container is the foreground PID 1 (tini → entrypoint → hope-agent); `server status` is designed for systemd / launchd-registered background services and doesn't apply here. Use `docker logs` or HEALTHCHECK instead.

**Forgot the Owner Token?** Run `docker compose exec hope-agent hope-agent server token show`. If an external secret owns the token, the command prints that effective value; never paste the output into logs or tickets.

## Forks and custom images

To run `.github/workflows/docker.yml` from a fork:

- In the fork's GitHub repo, go to Settings → Actions → General → Workflow permissions and enable `Read and write permissions` so `GITHUB_TOKEN` can push to GHCR
- The image name automatically becomes `ghcr.io/<your-username>/hope-agent` — the workflow uses `${{ github.repository_owner }}` dynamically

Or build locally:

```bash
docker buildx build \
  --platform linux/amd64,linux/arm64 \
  -t hope-agent:dev \
  --load \
  .
```

`--load` combined with multi-platform requires Docker Engine 23+ and the containerd image store.
