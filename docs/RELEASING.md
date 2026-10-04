# Releasing Factr-I

Current version: `0.0.0`. Releases live at
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
   | `macos-14` | `Factr-I-X.Y.Z-mac-arm64.dmg` and `.zip` |
   | `windows-latest` | `Factr-I-X.Y.Z-win-x64.exe` (NSIS installer) |
   | `ubuntu-latest` | `Factr-I-X.Y.Z-linux-*.AppImage` |

   The exact file names come from `artifactName` in `desktop/app/package.json`.
   Upload is done by `softprops/action-gh-release`; the build scripts themselves
   always run with `--publish never`.

Local equivalent of one OS leg: see `docs/BUILDING.md` (engine, then
`npm run stage:backend-python`, then `npm run dist:mac` / `dist:win:nsis`).

## Signing and notarization: not configured

The workflow builds unsigned artifacts (`CSC_IDENTITY_AUTO_DISCOVERY=false`).
Users will see Gatekeeper and SmartScreen warnings. To enable signing later, add
repository secrets and pass them to the Package step:

- macOS signing: `CSC_LINK` (base64 `.p12` Developer ID certificate) and
  `CSC_KEY_PASSWORD`, and remove `CSC_IDENTITY_AUTO_DISCOVERY=false`.
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

The desktop app checks GitHub; it never installs anything itself.

- `desktop/app/electron/update-feed.ts` (wired in `desktop/app/electron/main.ts`
  as the `factr:updates:check` and `factr:updates:apply` handlers) requests
  `https://api.github.com/repos/Refactr-io/Factr-I/releases/latest` and compares
  the tag with the running app version. Drafts and prereleases are ignored, and a
  404 (no release yet) means no update. Results are cached for 6 hours (a manual
  check is limited to once per 30 seconds), conditional requests use the ETag,
  and a request times out after 8 seconds. Failures such as a rate limit are
  reported as a check error, not as an update.
- When a newer release exists the app shows the version and the (truncated)
  release notes. "Apply" only opens the release page
  (`https://github.com/Refactr-io/Factr-I/releases`) in the browser. There is
  no auto-install, because the builds are unsigned; you install the new
  version yourself:
  - macOS: replace `Factr-I.app`. `desktop/app/scripts/factr-update.sh <new Factr-I.app> [installed app]`
    can do the swap: it keeps the old bundle as `Factr-I.app.previous`, launches
    the new one and waits (default 120 s, `FACTR_UPDATE_WAIT_S`) for it to write
    `launch-ok.json` with the bundle manifest id. If that never happens it
    restores the previous bundle and the pre-migration `factr.db` backup. It
    checks the engine binary against the sha256 in
    `resources/factr/manifest.json` and never touches `~/.factr`. It is not run
    automatically.
  - Windows: run the new NSIS installer over the old one (per-user install).
    There is no rollback script for Windows.
  - Linux: replace the AppImage.
- The app, the engine and the bundled Python runtime ship as one bundle and are
  replaced together.

Git-based update paths from earlier code (`electron/update-remote.ts`,
`factr_backend/banner.py`, which compare against the `Refactr-io/Factr-I` repo
over HTTPS) remain in the tree for source checkouts; they are not used by a
packaged build.
