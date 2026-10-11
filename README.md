<p align="center"><img src="docs/assets/factr-mark.png" alt="Factr-I" width="160"></p>

# Factr-I

A local-first AI agent: a Rust engine, a Python feature backend and an Electron
desktop app, built and shipped as one bundle.

## Layout

| Path | What |
| --- | --- |
| `engine/` | Rust workspace. Binary `factr` (crate `factr-cli`): agent runtime, sessions, memory, learning (`factr-learn`), and the desktop HTTP/WS gateway (`factr-gateway`). |
| `backend/` | Python feature backend (package `factr_backend`, console scripts `factr-backend`, `factr-acp`): tools, skills, cron, messaging platforms, plugins. |
| `desktop/` | npm workspace: `app/` (Electron + React desktop, `factr-desktop`), `shared/` (`@factr/shared`), `tests-js/`. |
| `scripts/` | Repository checks (`gate-names.sh`). |
| `docs/` | Notes for maintainers. |

## Quick start

```sh
# run from the repository root; each line is a subshell, so the directory does not change
(cd engine && cargo build --release -p factr-cli)       # engine: engine/target/release/factr
(cd backend && uv sync --extra dev)                     # backend (Python 3.11-3.13, uv)
(cd desktop && npm ci && npm --workspace app run typecheck)
```

## Build

Prerequisites (Rust 1.89+, MSVC Build Tools and Git for Windows on Windows, Node 22.22+/24.11+,
uv), the packaging steps for macOS (dmg, zip), Windows (NSIS) and Linux (AppImage), signing status
and every environment variable are in [docs/BUILDING.md](docs/BUILDING.md). Cutting a release and how
installed apps get updates: [docs/RELEASING.md](docs/RELEASING.md).

The current version is `v0.0.6`. CI is in `.github/workflows/ci.yml`, tagged releases in
`.github/workflows/release.yml` (macOS ad-hoc signed, not notarized; Windows and Linux unsigned).

## Installing a release

- macOS: Apple Silicon (M1 or later) only; Intel Macs are not supported. The app is ad-hoc signed but not
  notarized, so the first open is blocked. On macOS 15 and later: open Factr-I once and close the "Not Opened"
  dialog, then go to System Settings > Privacy & Security, click Open Anyway next to the Factr-I message, and
  confirm. Right-click > Open no longer bypasses this on macOS 15. Move the app to Applications before the first
  open: an app run from the disk image or Downloads runs from a read-only copy and cannot uninstall itself.
- Windows: x64 NSIS installer, not signed; SmartScreen warns (More info > Run anyway).
- Linux: x64 AppImage, not signed. `chmod +x Factr-I-*.AppImage`. On Ubuntu 24.04 install FUSE 2
  (`sudo apt install libfuse2t64`), and because AppArmor blocks the Chromium sandbox for AppImages, start it with
  `./Factr-I-*.AppImage --no-sandbox` (this turns off Chromium's renderer sandbox).

## Data locations

- `~/.factr/` - configuration, credentials and backend state (`FACTR_CONFIG_HOME`)
- `~/.factr/engine/` - engine state: sessions database `factr.db`, sockets (`FACTR_HOME`)
- engine settings: `~/.config/factr/` on Linux, `~/Library/Application Support/factr/` on macOS, `%APPDATA%\factr\` on Windows
- on Windows the desktop app uses `%LOCALAPPDATA%\factr` as the config home (an existing `%USERPROFILE%\.factr` is kept)

All environment variables use the `FACTR_` prefix.

## Before the first release

- The application id is `net.refactr.factr` (`desktop/app/package.json`, macOS bundle id).
  Use it only if you control the `refactr.net` domain / Apple team for that id; otherwise
  pick another reverse-DNS id before signing.
- There is no in-app installer for updates. The desktop app checks the latest GitHub release of
  `Refactr-io/Factr-I` (it reads the GitHub releases API) and, when a newer version exists, opens the release
  page so you can install it by replacing the app (see `docs/RELEASING.md`). Release builds have no Developer ID or
  Authenticode signature (macOS builds are ad-hoc signed).

## License

MIT, see `LICENSE`. Third-party notices: `THIRD_PARTY_NOTICES.md`.
