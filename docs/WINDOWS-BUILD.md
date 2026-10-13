# Building Factr-I for Windows (x64)

Source of truth: `.github/workflows/release.yml`, job `win-x64` (runs on `windows-latest`, and that job built the published 0.0.8 installer). This guide mirrors it step by step. It has not been run by hand on a developer Windows machine yet, so tell us about any step that differs.

Output: `desktop/app/release/Factr-I-<version>-win-x64.exe` (NSIS installer, unsigned; SmartScreen shows "More info > Run anyway").

Prebuilt installer: if you only need to run it, download `Factr-I-0.0.8-win-x64.exe` from https://github.com/Refactr-io/Factr-I/releases/tag/v0.0.8 (built by GitHub, unsigned). Build it yourself only to develop or verify.

## 1. Prerequisites

| Tool | Version | Why |
|---|---|---|
| Windows 10/11 x64 | any current | build host |
| Git for Windows | recent | checkout, bash shell for the steps below |
| Visual Studio Build Tools | 2022, workload "Desktop development with C++" | MSVC linker for Rust |
| Rust (rustup) | stable, target `x86_64-pc-windows-msvc` | the engine (`factr`) |
| NASM | recent, on PATH | needed by a C dependency of the engine |
| Node.js | 24 | desktop app and packaging |
| uv (astral) | recent | stages the bundled Python backend |

Enable long paths, because `node_modules` and the staged Python packages exceed 260 characters:

```bash
git config --global core.longpaths true
```

Also turn on Windows long paths (registry `LongPathsEnabled=1`) and clone to a short folder such as `C:\src\Factr-I`.

Run every command below in **Git Bash** (the workflow uses bash on Windows).

## 2. Get the source

```bash
git clone https://github.com/Refactr-io/Factr-I.git
cd Factr-I
git checkout v0.0.8
```

## 3. Build

Set the release flag for the whole session. Do NOT set any certificate variables (the build is unsigned):

```bash
export FACTR_RELEASE_BUILD=1
export CSC_IDENTITY_AUTO_DISCOVERY=false
```

1. Engine (Rust, release build, path-remapped so no local paths are embedded):

```bash
cd desktop/app
export FACTR_BUILD_SEMVER=0.0.8
node scripts/build-engine.mjs --locked -p factr-cli
```

2. Desktop dependencies (from the `desktop` folder, not `desktop/app`):

```bash
cd ..
npm ci
```

3. Stage the bundled Python backend:

```bash
cd app
npm run stage:backend-python
```

4. Package the installer:

```bash
npm run dist:win:nsis
```

The installer lands in `desktop/app/release/`.

## 4. Check the result

- The file is named `Factr-I-0.0.8-win-x64.exe`.
- Compute its checksum and compare it with the SHA256 digest GitHub shows next to the asset on the release page if you built the same tag: `certutil -hashfile Factr-I-0.0.8-win-x64.exe SHA256`. Builds are made reproducible with `SOURCE_DATE_EPOCH` and path remapping on macOS; on Windows a matching hash is not guaranteed, so a different hash alone is not a failure.
- Voice dictation downloads its speech engine on first use (about 200 MB, into your data folder). Expect about a minute the first time, then it is fast.
- Install on a clean Windows VM, open the app, sign in with ChatGPT or paste an API key, and send one chat.

## 4a. Known Windows limits (0.0.8)

- The installer is unsigned: SmartScreen warns.
- The release app has the Electron fuses that disable `RunAsNode` and the Node CLI/inspect options. The terminal pane on Windows (ConPTY) has not been tested with those fuses off. If the terminal pane fails to start on your build, report it with the app's log file (look under `%APPDATA%` and `%USERPROFILE%\.factr\logs`; the exact path is not verified on Windows).
- The Chromium remote debugging switches are blocked in the release build. For debugging use a QA build (`FACTR_QA_FUSES=1`, never ship it).

## 5. Troubleshooting

| Symptom | Fix |
|---|---|
| Path too long errors | long paths enabled in git and the registry, clone to a short path |
| `link.exe` not found | install the VS Build Tools C++ workload, reopen the shell |
| NASM errors in the engine build | install NASM and put it on PATH |
| `npm ci` fails with an old Node | use Node 24 |
| `stage:backend-python` cannot find uv | install uv and reopen the shell |
| `assert-root-install` error in `npm run build` | run `npm ci` in `desktop`, not in `desktop/app` |

## 6. Releasing from CI instead

Pushing a tag `v*` runs the same job on a GitHub Windows runner and attaches the installer to the release, so a local Windows build is for development and verification, not required for a release.
