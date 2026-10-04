# Building Factr-I

Factr-I is three parts built separately and shipped as one desktop bundle:

| Part | Where | Output |
| --- | --- | --- |
| Engine (Rust) | `engine/` | `engine/target/release/factr` (`factr.exe` on Windows) |
| Feature backend (Python) | `backend/` | a Python runtime plus packages staged for the app |
| Desktop app (Electron) | `desktop/app/` | `desktop/app/release/` |

All commands below were read from the scripts and config in this tree
(`engine/Cargo.toml`, `backend/pyproject.toml`, `desktop/package.json`,
`desktop/app/package.json` and `desktop/app/scripts/`). The version is `0.0.0`
in every one of them.

Verification status: this document was written by reading code on macOS. The
macOS flow is the one that has been exercised; the Windows and Linux flows are
derived from the scripts and the CI/release workflows (`.github/workflows/`)
and have not been run by the author. Where something is a guess it says so.

## Prerequisites

| Tool | Version | Notes |
| --- | --- | --- |
| Rust (stable, via rustup) | 1.89 or newer | There is no `rust-toolchain` file. The engine is edition 2024, uses let-chains and `File::try_lock` (config file locking), which need 1.89+. The tree was last built with 1.98. |
| CMake | any recent | Needed by `aws-lc-sys` (pulled in by rustls via the Bedrock provider). |
| NASM | any recent | Windows only, also for `aws-lc-sys`. CI installs it with `ilammy/setup-nasm`. |
| Node.js | `^22.22.0`, `^24.11.0` or `>=26.0.0` | From `engines` in `desktop/package.json` and `desktop/app/package.json`. CI uses 24. npm must be `<11.10.0` or `>=11.17.0`. |
| Python | 3.11 to 3.13 for development | `requires-python = ">=3.11,<3.14"` in `backend/pyproject.toml`. The packaged runtime is a bundled CPython 3.12.12 that `uv` downloads (see staging below). |
| uv | recent | Backend environment, and the packaging step uses it to fetch the bundled Python. |
| Git | any recent | Required: the staging step runs `git archive`, and the build embeds the commit hash. |
| ripgrep (`rg`) | optional | Only for `scripts/gate-names.sh`. |

### Windows

- Visual Studio 2022 Build Tools, "Desktop development with C++" workload
  (MSVC and a Windows SDK). Rust's default `x86_64-pc-windows-msvc` target needs it.
- Git for Windows. Run the commands below from PowerShell or Git Bash.
- CMake and NASM on `PATH`.
- Build Windows packages on Windows: `build-hud-modifier-monitor.mjs` compiles a
  helper with the .NET Framework `csc.exe` that ships with Windows, and refuses to
  cross-package for `win32`.
- Windows x64 is the only Windows target the staging script supports.

### macOS

- Xcode Command Line Tools (`xcode-select --install`). Packaging compiles a
  small Objective-C helper with `xcrun clang` and checks the engine and Python
  with `lipo`.

### Linux

- A C compiler (`cc`) and `make`. Packaging compiles a small native helper with
  `cc`. AppImage only needs what electron-builder downloads. Building `.deb` or
  `.rpm` (the other targets in the config) additionally needs `fpm`/`rpmbuild`.
- Only Linux x64 is supported by the staging script.

## 1. Engine

```sh
cd engine
cargo build --release -p factr-cli
```

This produces `engine/target/release/factr` (`factr.exe` on Windows). The
lockfile `engine/Cargo.lock` is committed; add `--locked` to refuse changes to it.

Release flavour: the desktop script `npm run build:engine` (from `desktop/app`)
runs the same build with `FACTR_RELEASE_BUILD=1`, which makes the embedded
version `v0.0.0 (<hash>)` instead of `v0.0.0-dev (<hash>)`.

Tests:

```sh
cd engine
cargo test --workspace --locked
```

Version: `factr --version` prints `Factr-I <version> (<sha>)` and
`factr __version` prints JSON `{version, sha, db_schema}`. `version` is the
`[package].version` of `engine/Cargo.toml` (`CARGO_PKG_VERSION`), currently
`0.0.0`; the packager records it in the bundle manifest.

## 2. Backend

```sh
cd backend
uv sync --extra dev        # creates backend/.venv (Python 3.11 to 3.13)
uv run pytest -q tests/test_base_url_hostname.py   # example single test file
```

`uv.lock` is committed; `uv sync --locked` fails if it is out of date. The full
suite is large; `.github/workflows/ci.yml` runs a small fast subset.

## 3. Desktop app

The workspace root is `desktop/` (workspaces `app`, `shared`, `tests-js`).
Dependencies are hoisted into `desktop/node_modules`, so always install from
`desktop/`:

```sh
cd desktop
npm ci
npm --workspace app run typecheck
npm --workspace app test          # vitest: the "ui" and "electron" projects
npm --workspace tests-js test     # repository-level checks (lockfile, entitlements)
```

`npm run build` in `desktop/app` runs `scripts/assert-root-install.mjs` first
and tells you to `npm ci` from `desktop/` if the install is partial.

Development: `cd desktop/app && npm run dev` (Vite on 127.0.0.1:5174 plus
Electron). To point the dev app at a backend checkout set
`FACTR_DESKTOP_BACKEND_ROOT`.

## 4. Staging the bundled Python and the engine

The packaged app carries its own Python runtime, the backend source and the
engine binary. `electron-builder` copies these from `desktop/app/build/`
(`extraResources` in `desktop/app/package.json`):

| Source | Goes to `resources/` | Produced by |
| --- | --- | --- |
| `build/backend-python/` | `backend-python/` | `npm run stage:backend-python` |
| `build/factr/` | `factr/` (engine plus `manifest.json`) | `scripts/before-pack.mjs`, during packaging |
| `build/install-stamp.json` | `install-stamp.json` | `scripts/write-build-stamp.mjs`, during `npm run build` |

Order matters: build the engine, stage the Python, then package.

```sh
cd engine && cargo build --release -p factr-cli          # 1. engine
cd ../desktop/app && npm run stage:backend-python        # 2. Python runtime + backend
```

`scripts/stage-backend-python.mjs` (supports macOS arm64/x64, Linux x64 and
Windows x64):

1. runs `uv python install 3.12.12 --install-dir build/backend-python-managed`
   (skipped when `FACTR_BACKEND_PYTHON_RUNTIME` is set);
2. copies that runtime to `build/backend-python/runtime`;
3. runs `uv pip install --target build/backend-python/packages --requirements backend/pyproject.toml --extra bundled`
   (skipped when `FACTR_BACKEND_PYTHON_PACKAGES` is set; installs the core
   `dependencies` plus the `bundled` extra, see [BUNDLED-EXTRAS.md](BUNDLED-EXTRAS.md);
   an import probe then fails the stage if any bundled package is missing);
4. extracts the backend source with `git archive HEAD:backend` into
   `build/backend-python/source`. This is the committed `HEAD` only: commit
   your backend changes first or they are not staged;
5. writes `defaults.json`, copies the `uv` binary to `tools/`, and writes
   `stage.json` (commit sha, dirty flag, Python version).

The engine is found by `before-pack.mjs` at `engine/target/release/factr` (or
`factr.exe`) when the target arch equals the host arch, or at
`engine/target/<triple>/release/` for a cross arch. Overrides: `FACTR_BIN`
(a specific binary) and `FACTR_ENGINE_ROOT` (a different `engine/` directory).
On macOS it checks with `lipo` that the engine and the bundled Python contain
the target architecture. It then writes `build/factr/manifest.json`, which the
app checks at launch.

## 5. Packaging

Run from `desktop/app`. `npm run build` (renderer, Electron main, build stamp,
native deps) is part of each of these.

| Command | Result |
| --- | --- |
| `npm run pack` | Unpacked app only (`electron-builder --dir`): `release/mac-arm64/Factr-I.app` (or `mac/`), `release/win-unpacked/`, `release/linux-unpacked/`. Fast check, no installer. |
| `npm run dist:mac` | macOS `.dmg` and `.zip` for the host arch. `dist:mac:dmg` and `dist:mac:zip` build one of them. |
| `npm run dist:win:nsis` | Windows NSIS installer (`Factr-I-<version>-win-x64.exe`). `dist:win` builds every Windows target in the config (`nsis` and `msi`); `dist:win:msi` only the MSI. |
| `npm run dist:linux` | Linux `AppImage`, `deb` and `rpm`. For the AppImage alone: `npm run build && npm run builder -- --linux AppImage`. |

Artifact names follow `Factr-I-${version}-${os}-${arch}.${ext}` and everything
lands in `desktop/app/release/`. Cross-building is not supported: build each OS
on that OS.

`npm run builder` always passes `--publish never`; nothing is uploaded by these
scripts.

### Signing

None is configured. Builds are unsigned and not notarized.

- macOS: `scripts/notarize.mjs` is wired as `afterSign` and skips itself unless
  `APPLE_NOTARY_PROFILE`, or all of `APPLE_API_KEY`, `APPLE_API_KEY_ID` and
  `APPLE_API_ISSUER`, are set. Signing itself uses the standard electron-builder
  variables (`CSC_LINK`, `CSC_KEY_PASSWORD`, or a keychain identity). Set
  `CSC_IDENTITY_AUTO_DISCOVERY=false` to build unsigned deliberately.
- Windows: `win.signAndEditExecutable` is `false`; no certificate is wired.
  electron-builder would read `WIN_CSC_LINK` and `WIN_CSC_KEY_PASSWORD`.
- An unsigned macOS app is blocked by Gatekeeper on other machines
  (right-click, Open, or `xattr -dr com.apple.quarantine Factr-I.app`); an
  unsigned Windows installer shows SmartScreen.

The application id is `net.refactr.factr`.

## Data directories and environment

| What | macOS / Linux | Windows |
| --- | --- | --- |
| Config home (`FACTR_CONFIG_HOME`): config, credentials, backend state, logs | `~/.factr/` | `%USERPROFILE%\.factr` (same as the standalone engine, which keeps its own data under `.factr\engine`) |
| Engine home (`FACTR_HOME`): `factr.db`, sockets, logs | `~/.factr/engine/` | `%USERPROFILE%\.factr\engine\` |
| Engine settings | Linux `~/.config/factr/`, macOS `~/Library/Application Support/factr/` | `%APPDATA%\factr\` (from the `dirs` crate); `<FACTR_HOME>/config/factr` when `FACTR_HOME` is set |
| Electron user data | macOS `~/Library/Application Support/Factr-I` | `%APPDATA%\Factr-I` |

Electron user data (`Factr-I`) and the engine settings folder (`factr`) are different names on purpose: macOS and Windows are case-insensitive, so neither may be renamed or merged into the other.

Variables used by the build and the app (all `FACTR_` prefixed unless noted):

| Variable | Used by | Meaning |
| --- | --- | --- |
| `FACTR_CONFIG_HOME` | engine, backend, desktop | Config home (above). |
| `FACTR_HOME` | engine, desktop | Engine data directory. Setting it sandboxes engine state, config and external auth files beneath it. |
| `FACTR_PROFILE` | engine | Set by `--profile <name>`. |
| `FACTR_RELEASE_BUILD` | `build.rs` | `1` embeds a release version string. Set by `npm run build:engine`. |
| `FACTR_BUILD_SEMVER` | `build.rs` | Override the embedded semver (the release workflow sets it from the tag). |
| `FACTR_BIN`, `FACTR_ENGINE_ROOT` | `before-pack.mjs` | Which engine binary or `engine/` directory to package. |
| `FACTR_BACKEND_PYTHON_RUNTIME`, `FACTR_BACKEND_PYTHON_PACKAGES` | `stage-backend-python.mjs` | Use a prepared Python runtime or package directory instead of downloading. |
| `FACTR_DESKTOP_BACKEND_ROOT` | dev app | Run against a backend checkout. |
| `FACTR_DESKTOP_USER_DATA_DIR` | desktop | Throwaway user-data and config home (used by `test:desktop:fresh`). |
| `FACTR_PROVIDER`, `FACTR_MODEL`, `FACTR_OLLAMA_NUM_CTX` | packaged app | Provider, model and Ollama context size passed to the engine. |
| `FACTR_UPDATE_WAIT_S`, `FACTR_UPDATE_USERDATA` | `factr-update.sh` | See `docs/RELEASING.md`. |
| `APPLE_*`, `CSC_*`, `WIN_CSC_*` | signing | Not Factr variables; see Signing. |

## Name gate

```sh
bash scripts/gate-names.sh
```

Needs `rg`. Must print zero hits; CI runs it.

## Known gaps

- The Windows and Linux flows above are unverified on real machines.
- Some engine tests shell out to `sh`/`bash` and `/dev/zero`; the Windows CI job
  runs `cargo test` as non-blocking until that is checked.
- `desktop/app/scripts/factr-update.sh` (the update script) is macOS only.
