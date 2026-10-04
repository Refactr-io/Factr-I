# Maintainer TODO

- Regenerate the lockfiles once on a build machine: run `npm install` in
  `desktop/` to drop entries of workspaces that are not part of this repository,
  and `uv lock` in `backend/`.
- Code signing and notarization are not configured (see `docs/RELEASING.md`).
- Verify the Windows and Linux build flows and the Windows engine tests on real
  machines (see "Known gaps" in `docs/BUILDING.md`).
- Windows has no update rollback script (macOS has `factr-update.sh`).
