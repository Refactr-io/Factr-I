# Releasing Factr-I

Current version: `0.0.14`. Releases live at
https://github.com/Refactr-io/Factr-I/releases (the `publish` block of
`desktop/app/package.json` and every update link in the app point there).

## Cutting a release

1. Pick the version `X.Y.Z` and set it everywhere it is stored. These must all
   agree; the release workflow refuses to build when the tag does not match the
   first three:
   - `engine/Cargo.toml` `[package].version` (this is what `factr __version` and
     `factr --version` report), and the `factr-cli` entry in `engine/Cargo.lock`
   - `backend/pyproject.toml` `version`, the `factr-backend` entry in
     `backend/uv.lock`, and `__version__` in `backend/factr_backend/__init__.py`
     (the About panel reads that file)
   - `desktop/app/package.json` `version` (it names the installer files) and the
     `app` entry of `desktop/package-lock.json`
2. Make sure CI is green on that commit (`.github/workflows/ci.yml`) and
   `bash scripts/gate-names.sh` reports no hits.
3. Commit, then tag and push the tag:

   ```sh
   git tag vX.Y.Z
   git push origin vX.Y.Z
   ```

   The tag must be exactly `vMAJOR.MINOR.PATCH`.
4. `.github/workflows/release.yml` runs on the tag. It builds the engine in
   release mode on each OS (with `FACTR_RELEASE_BUILD=1` and
   `FACTR_BUILD_SEMVER=X.Y.Z`), stages the bundled Python, packages the app and
   creates the GitHub Release with auto-generated notes:

   | Runner | Artifact |
   | --- | --- |
   | `macos-14` | `Factr-I-X.Y.Z-mac-arm64.dmg` and `.zip` (Apple Silicon only; ad-hoc signed and verified with `codesign --verify --deep --strict`) |
   | `windows-latest` | `Factr-I-X.Y.Z-win-x64.exe` (NSIS installer) |
   | `ubuntu-latest` | `Factr-I-X.Y.Z-linux-*.AppImage` |

   The exact file names come from `artifactName` in `desktop/app/package.json`.
   Upload is done by `softprops/action-gh-release`; the build scripts themselves
   always run with `--publish never`.

Local equivalent of one OS leg: see `docs/BUILDING.md` (engine, then
`npm run stage:backend-python`, then `npm run dist:mac` / `dist:win:nsis`).

## Signing and notarization: not configured

There is no Developer ID or Authenticode certificate. The macOS app is ad-hoc
signed (`-c.mac.identity=-`, so it is not reported as "damaged" on Apple
Silicon) but not notarized; the Windows and Linux artifacts are unsigned. Users
see Gatekeeper on the first open (macOS 15+: System Settings > Privacy &
Security > Open Anyway; right-click > Open no longer works) and SmartScreen on
Windows. The release notes repeat these steps and the Linux AppImage notes (FUSE 2,
`--no-sandbox` on Ubuntu 24.04). To enable signing later, add repository secrets
and pass them to the Package step:

- macOS signing: `CSC_LINK` (base64 `.p12` Developer ID certificate) and
  `CSC_KEY_PASSWORD`, and drop `-c.mac.identity=-` from the mac build command.
- macOS notarization: `APPLE_API_KEY` (the `.p8` contents), `APPLE_API_KEY_ID` and
  `APPLE_API_ISSUER`; `desktop/app/scripts/notarize.mjs` already uses them and
  skips itself when they are absent.
- Windows signing: `WIN_CSC_LINK` and `WIN_CSC_KEY_PASSWORD`, and set
  `build.win.signAndEditExecutable` to `true` in `desktop/app/package.json`
  (it is `false` now). The Windows icon stamping hook
  (`scripts/after-extract.mjs`) is independent of this.

The macOS application id `net.refactr.factr` should only be signed by whoever
controls that identity.

## How the app learns about updates

The desktop app checks GitHub and, on Update now, downloads the installer for its platform and swaps itself in place.

- `desktop/app/electron/update-feed.ts` (wired in `desktop/app/electron/main.ts`
  as the `factr:updates:check` and `factr:updates:apply` handlers) requests
  `https://api.github.com/repos/Refactr-io/Factr-I/releases/latest` and compares
  the tag with the running app version. Drafts and prereleases are ignored, and a
  404 (no release yet) means no update. Results are cached for 6 hours (a manual
  check is limited to once per 30 seconds), conditional requests use the ETag,
  and a request times out after 8 seconds. Failures such as a rate limit are
  reported as a check error, not as an update, and the app does not ask again for
  15 minutes (30 seconds for a manual check), or longer when GitHub sends
  `Retry-After` or an exhausted `x-ratelimit-reset`.
- When a newer release exists the app shows the version and the (truncated)
  release notes. Update now (the `factr:updates:apply` handler) downloads the
  installer asset for the platform from GitHub over HTTPS (host allow-list,
  checked on every redirect), refuses it unless its size and sha256 match what
  the release lists, and then replaces the installed app, with the Factr-I
  update screen (`update-splash.ts`) on top. Nothing starts while an agent turn
  is running. The builds are not signed with a Developer ID or Authenticode
  certificate, so there is no Squirrel / electron-updater; each platform has its
  own helper that waits for the app to quit, swaps, relaunches and waits for the
  new app to write a health marker (`<userData>/updates/<version>/ok`, written
  only by an app whose version is the one it was updated to), and otherwise
  restores the old install:
  - macOS (`electron/update-swap.ts`): unzips the verified zip next to the app,
    checks the bundle id, version, architecture and `codesign --verify`, then a
    detached shell helper renames old to `.old` and new into place. A copy run
    from a disk image or an App Translocation folder is not replaced.
  - Windows (`electron/update-swap-other.ts`): copies the install folder to
    `<install>.previous`, then a detached PowerShell helper waits for every
    process in the install folder to exit, runs the NSIS installer with `/S`
    into the same folder, relaunches, and restores the copy if the installer or
    the new app fails. A copy that is not an NSIS install is not replaced.
  - Linux (`electron/update-swap-other.ts`): stages the AppImage as a hidden
    sibling of the running one (`$APPIMAGE`), then a detached shell helper keeps
    the old image as a hard link, renames the new one over it, relaunches and
    puts the old one back if the new app does not report healthy. A copy that is
    not an AppImage (deb, rpm) is not replaced.
  Whenever an in-place update is not possible the app falls back to the old
  behaviour (Windows opens the installer, Linux shows the AppImage, the other
  cases open the release page) and says so.
  `.github/workflows/update-e2e.yml` proves this on real Windows and Linux
  runners: it installs the old build, serves the new one from a local fake
  release feed, presses Update now and checks the new version came up healthy;
  a second run serves a corrupted download and expects a clean refusal.
- The app, the engine and the bundled Python runtime ship as one bundle and are
  replaced together.

Git-based update paths from earlier code (`electron/update-remote.ts`,
`factr_backend/banner.py`, which compare against the `Refactr-io/Factr-I` repo
over HTTPS) remain in the tree for source checkouts; they are not used by a
packaged build.
