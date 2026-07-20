# syntax=docker/dockerfile:1.7
#
# Hope Agent — multi-arch container image for `hope-agent server`.
#
# Stage 1 (web)     — node:20-bookworm-slim builds the Vite frontend to `/work/dist/`.
#                     Pinned to $BUILDPLATFORM so we run pnpm exactly once even
#                     for multi-arch builds (frontend output is arch-agnostic).
#                     Node stage doesn't have the glibc constraint below.
# Stage 2 (rust)    — rust:1.95.0-trixie builds the `hope-agent` binary.
#                     The web `dist/` is copied in BEFORE `cargo build` so
#                     `crates/ha-server/build.rs` sees the real assets and
#                     `rust-embed` bakes them into the binary.
# Stage 3 (runtime) — debian:trixie-slim — ca-certs + tzdata + wget + a few
#                     desktop-tool shared libs (see runtime stage comment).
#                     Runs as non-root user `hope` (uid 1000) with /data
#                     persisted as the configurable HA_DATA_DIR.
#
# Glibc note: trixie (Debian 13, glibc 2.41) on both build and runtime is
# required because `ort-sys` (pulled in by fastembed for embeddings) ships
# prebuilt ONNX Runtime binaries that reference `__isoc23_strtol` from
# glibc 2.38+. bookworm (glibc 2.36) fails to link. release.yml already
# targets ubuntu-24.04 (glibc 2.39), so this is consistent with the bare-
# binary baseline. Keep both stages on the same distro — mixing trixie
# build / bookworm runtime would dynamically fail to load at startup.

# -------------------------------------------------------------------
# Stage 1: build the Vite frontend (arch-independent)
# -------------------------------------------------------------------
FROM --platform=$BUILDPLATFORM node:20-bookworm-slim AS web

# Pin pnpm to the version declared in package.json#packageManager so
# lockfile resolution is reproducible. `corepack prepare --activate`
# downloads the pinned tarball; `pnpm-lock.yaml` was generated with the
# same version.
ENV COREPACK_DEFAULT_TO_LATEST=0 \
    HUSKY=0
RUN corepack enable && corepack prepare pnpm@10.33.1 --activate

WORKDIR /work

# Install dependencies in a separate layer keyed on lockfile changes so
# editing source files doesn't trigger a full `pnpm install`.
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml .npmrc ./
RUN --mount=type=cache,target=/root/.local/share/pnpm/store \
    pnpm install --frozen-lockfile --ignore-scripts

# Copy the frontend sources and build.
COPY index.html vite.config.ts tsconfig.json tsconfig.app.json tsconfig.node.json eslint.config.js ./
COPY src ./src
COPY public ./public
COPY scripts ./scripts

# Docker installs dependencies with --ignore-scripts, so apply the
# CodeMirror EditContext patch explicitly before bundling the web UI.
RUN node scripts/patch-codemirror-edit-context.mjs

RUN pnpm build && \
    # Sanity check — the rust stage assumes /work/dist/index.html exists
    # and is the REAL build output, not `crates/ha-server/build.rs`'s
    # placeholder page.
    test -s dist/index.html

# -------------------------------------------------------------------
# Stage 2: build the Rust `hope-agent` binary
# -------------------------------------------------------------------
FROM rust:1.95.0-trixie AS rust

# protobuf-compiler is required by `prost-build` at compile time.
# pkg-config is needed by several -sys crates even though OpenSSL is
# vendored.
# libclang-dev is required by bindgen (pulled in by `libspa-sys`-style
# crates that may still appear transitively; harmless when unused).
#
# Desktop-only image tools (`xcap` for screen capture, `arboard` for
# clipboard) are gated behind ha-core's `desktop-tools` Cargo feature,
# which we do NOT enable here — keeping wayland / pipewire / gtk-3 out
# of both the build deps and the runtime libs.
RUN apt-get -o Acquire::Retries=5 -o Acquire::http::Timeout=60 update && \
    apt-get -o Acquire::Retries=5 -o Acquire::http::Timeout=60 install -y --no-install-recommends \
        pkg-config \
        protobuf-compiler \
        libclang-dev \
        mold \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /work

# Copy the workspace metadata first so dependency compilation is cached
# independently of source-only edits.
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates/ha-core/Cargo.toml crates/ha-core/Cargo.toml
COPY crates/ha-server/Cargo.toml crates/ha-server/Cargo.toml
COPY src-tauri/Cargo.toml src-tauri/Cargo.toml

# Now copy the actual source.
COPY crates ./crates
COPY src-tauri ./src-tauri
# ha-core embeds three trees from outside crates/ via `rust-embed` at compile
# time (the derive hard-errors if the folder is missing): the bundled skills
# (`skills/embedded.rs`), the Chrome extension runtime files
# (`browser/extension/embedded.rs`), and the bilingual user manual
# (`manual/embed.rs`). The binary materializes them under the data dir on
# first use, so the runtime stage ships no separate copies.
COPY skills ./skills
COPY extensions ./extensions
COPY docs/user-guide ./docs/user-guide
# crates/ha-core/src/agent_loader.rs include_bytes!s the frontend logo at
# compile time. (skills/ and extensions/ below are the other out-of-crate
# trees the Rust build embeds.)
COPY src/assets ./src/assets

# Critical: the frontend dist MUST be present before `cargo build` runs,
# otherwise `crates/ha-server/build.rs` (called by cargo) writes its
# placeholder index.html and rust-embed bakes that into the binary
# permanently.
COPY --from=web /work/dist ./dist

# Build only the headless `hope-agent-server` binary shipped from the
# `ha-server` crate. The Tauri-built `hope-agent` binary in `src-tauri`
# is intentionally skipped — it would pull in WebKit / Cocoa / WinRT.
# The bin is named `hope-agent-server` upstream to avoid colliding with
# src-tauri's `hope-agent` in `target/release/`; we rename it back to
# `hope-agent` on the copy so the in-container command stays unchanged.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/work/target \
    RUSTFLAGS="-C link-arg=-fuse-ld=mold" \
    cargo build --release --locked -p ha-server --bin hope-agent-server && \
    cp /work/target/release/hope-agent-server /usr/local/bin/hope-agent

# -------------------------------------------------------------------
# Stage 3: minimal runtime
# -------------------------------------------------------------------
FROM debian:trixie-slim AS runtime

# ca-certificates: required for outbound HTTPS to provider APIs.
# tzdata: required for cron schedules / `TZ` env var to take effect.
# wget: used by HEALTHCHECK below.
# tini: PID 1 with proper signal forwarding so `docker stop` shuts the
#       hope-agent server down cleanly.
# chromium + shared libs: makes `profile.op=launch headless=true` work
#       out of the box. hope-agent's `find_chrome_executable()` probes
#       `chromium` first in PATH; without it the agent would have to
#       fall back to runtime download (~150 MB) on first browser call.
#       Users who don't need the browser tool can rebuild without this
#       block to shave ~250 MB off the image.
#
# No wayland / pipewire / gtk-3 / egl libs here — the desktop-tools
# Cargo feature is disabled when ha-server builds `hope-agent`, so xcap /
# arboard never get linked in and their runtime dependencies aren't
# needed. See `crates/ha-core/Cargo.toml` `[features]` for the gate.
RUN apt-get -o Acquire::Retries=5 -o Acquire::http::Timeout=60 update && \
    apt-get -o Acquire::Retries=5 -o Acquire::http::Timeout=60 install -y --no-install-recommends \
        ca-certificates \
        tzdata \
        wget \
        tini \
        chromium \
        fonts-liberation \
        libnss3 \
        libgbm1 \
        libxss1 \
    && rm -rf /var/lib/apt/lists/*

# Non-root user. /data is the persisted HA_DATA_DIR (mount this as a volume).
RUN groupadd --system --gid 1000 hope && \
    useradd  --system --uid 1000 --gid hope --shell /bin/sh --home-dir /data --create-home hope

COPY --from=rust /usr/local/bin/hope-agent /usr/local/bin/hope-agent
COPY docker/docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh
RUN chmod +x /usr/local/bin/docker-entrypoint.sh

ENV HA_DATA_DIR=/data \
    HA_DEPLOYMENT=docker \
    HA_BIND=0.0.0.0:8420 \
    TZ=UTC

USER hope
WORKDIR /data
EXPOSE 8420

# `/api/health` is unauthenticated (see ha-server/middleware.rs) so it
# can be hit from inside the container regardless of the API key state.
HEALTHCHECK --interval=30s --timeout=5s --start-period=15s --retries=3 \
    CMD wget -qO- http://127.0.0.1:8420/api/health >/dev/null 2>&1 || exit 1

ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/docker-entrypoint.sh"]
CMD ["server", "start"]
