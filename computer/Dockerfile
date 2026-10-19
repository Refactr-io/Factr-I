# syntax=docker/dockerfile:1.7
# Factr-I bot computer: one bot's engine, Python feature backend, Xfce desktop and Chromium.
# Build from the repo root:  docker build -f computer/Dockerfile -t bots-factr-bot-computer:dev .

ARG RUST_IMAGE=rust:1.98-bookworm
ARG PYTHON_VERSION=3.12.12

# ---- Stage 1: the Rust engine (`factr`) -----------------------------------------------------
FROM ${RUST_IMAGE} AS engine
ARG FACTR_BUILD_GIT_HASH=unknown
ENV FACTR_BUILD_GIT_HASH=${FACTR_BUILD_GIT_HASH} CARGO_INCREMENTAL=0
RUN apt-get update \
 && apt-get install -y --no-install-recommends cmake clang pkg-config libssl-dev \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /src/engine
COPY engine/ ./
# Same flags as desktop/app/scripts/build-engine.mjs: no build-machine paths in the binary.
ENV CARGO_ENCODED_RUSTFLAGS="--remap-path-prefix=/src/engine=/factr/engine"
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/src/engine/target \
    cargo build --release --bin factr \
 && install -D target/release/factr /out/factr

# ---- Stage 2: the Python feature backend, laid out like the packaged app's backend-python/ ----
FROM debian:bookworm-slim AS backend
ARG PYTHON_VERSION
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl python3-pip python3-venv build-essential \
 && rm -rf /var/lib/apt/lists/* \
 && pip install --no-cache-dir --break-system-packages uv
ENV UV_PYTHON_INSTALL_DIR=/opt/pybuild UV_LINK_MODE=copy
RUN uv python install ${PYTHON_VERSION}
WORKDIR /opt/backend-python
# runtime/: the interpreter (python3.12 inside), packages/: the `bundled` extra, source/, tools/uv, defaults.json
RUN cp -aL $(dirname $(dirname $(uv python find ${PYTHON_VERSION}))) runtime \
 && mkdir packages source tools
COPY backend/pyproject.toml /tmp/pyproject.toml
RUN --mount=type=cache,target=/root/.cache/uv \
    uv pip install --target packages --python runtime/bin/python3.12 \
        --requirements /tmp/pyproject.toml --extra bundled
COPY backend/ source/
RUN rm -rf source/tests && cp "$(command -v uv)" tools/uv
ENV PYTHONPATH=/opt/backend-python/source:/opt/backend-python/packages PYTHONDONTWRITEBYTECODE=1
RUN runtime/bin/python3.12 -c "import json; from factr_backend.config_defaults import DEFAULT_CONFIG; \
d=dict(DEFAULT_CONFIG); d['auth']={**d.get('auth',{}),'adopt_external_logins':False}; print(json.dumps(d))" > defaults.json \
 && runtime/bin/python3.12 -c "import factr_bootstrap, aiohttp, anthropic, mcp, psutil" \
 && { runtime/bin/python3.12 -m compileall -q -f -j 0 --invalidation-mode unchecked-hash \
        runtime packages source >/dev/null || [ $? -eq 1 ]; }

# ---- Stage 3: runtime -----------------------------------------------------------------------
FROM debian:bookworm-slim AS runtime
# Bot Desktop packages: backend/tools/bot_desktop/runtime.py PACKAGES["apt"] (REQUIRED_BINARIES),
# plus the browser, fonts, tini and the basics the agent expects.
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      tigervnc-standalone-server xfce4-panel xfwm4 xfdesktop4 xfce4-settings xfce4-terminal \
      dbus-x11 x11-xserver-utils x11-utils x11-xkb-utils xauth fonts-dejavu-core \
      chromium fonts-noto-cjk fonts-noto-color-emoji \
      tini git curl ca-certificates procps libssl3 socat tzdata \
 && rm -rf /var/lib/apt/lists/*
RUN useradd --uid 1000 --create-home --shell /bin/bash bot \
 && mkdir -p /data/profiles /tmp/.X11-unix \
 && chown -R bot:bot /data \
 && chmod 1777 /tmp/.X11-unix
COPY --from=engine /out/factr /usr/local/bin/factr
COPY --from=backend /opt/backend-python /opt/factr/backend-python
COPY computer/entrypoint.sh /usr/local/bin/factr-bot-entrypoint
RUN chmod 0755 /usr/local/bin/factr-bot-entrypoint
USER bot
WORKDIR /home/bot
ENV FACTR_CONFIG_HOME=/data \
    HOME=/home/bot \
    FACTR_BOT_COMPUTER=1 \
    FACTR_ENGINE_PORT=8642
EXPOSE 8642
HEALTHCHECK --interval=15s --timeout=3s --start-period=20s --retries=3 \
  CMD curl -fsS "http://127.0.0.1:${FACTR_ENGINE_PORT:-8642}/api/health" >/dev/null || exit 1
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/factr-bot-entrypoint"]
