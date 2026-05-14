#!/bin/sh
# Hope Agent container entrypoint.
#
# Translates a small set of environment variables to CLI flags before
# exec'ing the hope-agent binary. The binary itself only knows about
# command-line flags today (see src-tauri/src/main.rs::parse_server_args);
# entrypoint translation keeps the binary unchanged while still letting
# docker / compose / k8s users configure via env vars.
#
# Env vars honored:
#   HA_BIND        — bind address, default 0.0.0.0:8420 (set in Dockerfile ENV)
#   HA_API_KEY     — if set, passed as `--api-key`; empty string disables
#   HA_DATA_DIR    — data root, default /data (set in Dockerfile ENV;
#                    consumed by ha-core::paths)
#   HA_DEPLOYMENT  — `docker` (set in Dockerfile ENV; read by updater so
#                    `app_update install` redirects users to `docker pull`)
#
# CMD args are passed through, so `docker run ... server status` and
# similar admin invocations work without going through the translation.

set -eu

DATA_DIR="${HA_DATA_DIR:-/data}"

# Strip a stale server.pid left over by a previous container that was
# SIGKILLed (e.g. `docker rm -f`). Without this, `hope-agent server
# status` would report a phantom running instance.
# Path matches src-tauri/src/main.rs:415.
if [ -f "$DATA_DIR/server.pid" ]; then
    rm -f "$DATA_DIR/server.pid" 2>/dev/null || true
fi

# Only inject env-derived flags when the CMD targets the server.
# This way `docker run ... hope-agent server status` (or any other
# subcommand) still gets a clean argv.
should_translate=0
case "${1:-}" in
    server)
        case "${2:-}" in
            start|"") should_translate=1 ;;
        esac
        ;;
esac

if [ "$should_translate" -eq 1 ]; then
    set -- "$@" \
        ${HA_BIND:+--bind "$HA_BIND"} \
        ${HA_API_KEY:+--api-key "$HA_API_KEY"}
fi

exec hope-agent "$@"
