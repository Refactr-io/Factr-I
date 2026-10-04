#!/bin/sh
# Replace an installed Factr-I.app with a new build and roll back if the new one never gets healthy.
#
#   factr-update.sh <new Factr-I.app> [installed app, default /Applications/Factr-I.app]
#
# Engine, bundled Python runtime and Factr source live inside the .app (resources/factr*), so one
# swap replaces all three together; the previous bundle stays next to it as <app>.previous. The app writes
# <userData>/launch-ok.json with its manifest id once its engine answered (electron/main.ts), and that is the
# health check. User data (~/.factr, ~/.factr/engine, factr.db) is never touched; the engine backs up
# factr.db itself (factr.db.pre-vN.bak) before any schema migration. Signing is not involved.
#
# FACTR_UPDATE_WAIT_S: seconds to wait for the new build to report healthy (default 120; the old name
# FACTR_UPDATE_WAIT still works). It is doubled when the new manifest's engine.dbSchema (the factr.db
# migrations the engine runs at first start) is higher than the installed one's; a manifest without dbSchema,
# on either side, gets no automatic extension, so set FACTR_UPDATE_WAIT_S yourself for a big migration.
set -eu

NEW=${1:?usage: factr-update.sh <new Factr-I.app> [installed app]}
APP=${2:-/Applications/Factr-I.app}
USER_DATA=${FACTR_UPDATE_USERDATA:-$HOME/Library/Application Support/Factr}
OPEN=${FACTR_UPDATE_OPEN:-open}
WAIT=${FACTR_UPDATE_WAIT_S:-${FACTR_UPDATE_WAIT:-120}}
DB_DIR=${FACTR_UPDATE_DBDIR:-${FACTR_HOME:-$HOME/.factr/engine}}
RES=Contents/Resources

db_schema() { sed -n 's/.*"dbSchema": *\([0-9][0-9]*\).*/\1/p' "$1/$RES/factr/manifest.json" 2>/dev/null | head -n 1; }
manifest_id() { sed -n 's/.*"id": *"\([^"]*\)".*/\1/p' "$1/$RES/factr/manifest.json" 2>/dev/null | head -n 1; }

# An earlier update that was interrupted between moving the installed app aside and putting the new one in
# leaves no $APP but the old bundle at $APP.previous: put it back rather than delete it below.
if [ ! -d "$APP" ] && [ -d "$APP.previous" ]; then
  echo "restoring $APP from $APP.previous (an earlier update was interrupted)" >&2
  mv "$APP.previous" "$APP"
fi

for part in factr/factr factr/manifest.json backend-python/runtime; do
  [ -e "$NEW/$RES/$part" ] || { echo "refusing: $NEW is missing $RES/$part" >&2; exit 2; }
done
NEW_ID=$(manifest_id "$NEW")
# The manifest records the engine binary's sha256 at pack time; a bundle whose binary no longer matches
# (truncated copy, tampering) is refused before anything is swapped. Older manifests without it pass.
WANT=$(sed -n 's/.*"sha256": *"\([0-9a-f]*\)".*/\1/p' "$NEW/$RES/factr/manifest.json" | head -n 1)
if [ -n "$WANT" ]; then
  GOT=$(shasum -a 256 "$NEW/$RES/factr/factr" | cut -d' ' -f1)
  [ "$GOT" = "$WANT" ] || { echo "refusing: engine sha256 $GOT does not match the manifest ($WANT)" >&2; exit 2; }
fi
[ -n "$NEW_ID" ] || { echo "refusing: $NEW has no readable manifest id" >&2; exit 2; }
if [ -z "${FACTR_UPDATE_SKIP_RUNNING_CHECK:-}" ] && pgrep -f "$APP/Contents/MacOS" >/dev/null 2>&1; then
  echo "quit Factr first, then run the update again" >&2
  exit 3
fi

NEW_SCHEMA=$(db_schema "$NEW")
OLD_SCHEMA=$(db_schema "$APP")
if [ -n "$NEW_SCHEMA" ] && [ -n "$OLD_SCHEMA" ] && [ "$NEW_SCHEMA" -gt "$OLD_SCHEMA" ]; then
  WAIT=$((WAIT * 2)) # the first start migrates factr.db
fi

# A leftover pre-v*.bak from an earlier update would make the new engine skip its own (it backs up once
# per target version), and rollback would then find nothing newer than $STARTED. Only after the bundle passed its checks.
rm -f "$DB_DIR"/factr.db.pre-v*.bak

STARTED=$(mktemp)
trap 'rm -f "$STARTED"' EXIT
rm -rf "$APP.previous" "$APP.new"
ditto "$NEW" "$APP.new"
if [ -d "$APP" ]; then mv "$APP" "$APP.previous"; fi
mv "$APP.new" "$APP"

rm -f "$USER_DATA/launch-ok.json"
"$OPEN" "$APP" || true
i=0
while [ "$i" -lt "$WAIT" ]; do
  if grep -q "\"id\": *\"$NEW_ID\"" "$USER_DATA/launch-ok.json" 2>/dev/null; then
    echo "updated to build $NEW_ID; previous bundle kept at $APP.previous"
    exit 0
  fi
  i=$((i + 1))
  sleep 1
done

echo "build $NEW_ID did not report healthy within ${WAIT}s; rolling back" >&2
# The engine lives under Resources/, not MacOS/: stop both and wait for them to exit, or a dying engine
# can still checkpoint the WAL over the database we are about to restore.
ENGINE="$APP/Contents/Resources/factr/factr"
pkill -f "$APP/Contents/MacOS" 2>/dev/null || true
pkill -f "$ENGINE" 2>/dev/null || true
i=0
while pgrep -f "$APP/Contents/MacOS" >/dev/null 2>&1 || pgrep -f "$ENGINE" >/dev/null 2>&1; do
  i=$((i + 1))
  [ "$i" -gt 20 ] && pkill -9 -f "$ENGINE" 2>/dev/null
  [ "$i" -gt 30 ] && break
  sleep 1
done
# The new engine may have migrated factr.db and an older engine refuses a newer file: put back the
# backup it took during this update (newest pre-v*.bak newer than the start marker). Nothing else is touched.
BAK=
while IFS= read -r f; do
  if [ -z "$BAK" ] || [ "$f" -nt "$BAK" ]; then BAK=$f; fi
done <<EOF
$(find "$DB_DIR" -maxdepth 1 -name 'factr.db.pre-v*.bak' -newer "$STARTED" 2>/dev/null)
EOF
if [ -n "$BAK" ]; then
  rm -f "$DB_DIR/factr.db-wal" "$DB_DIR/factr.db-shm"
  cp "$BAK" "$DB_DIR/factr.db"
  # The engine skips its pre-migration backup when one exists, so a used backup would leave the next
  # attempt with nothing to roll back to.
  rm -f "$BAK"
  echo "restored factr.db from $BAK" >&2
fi
if [ -d "$APP.previous" ]; then
  rm -rf "$APP.failed"
  mv "$APP" "$APP.failed"
  mv "$APP.previous" "$APP"
  "$OPEN" "$APP" || true
  echo "restored the previous bundle; the failed one is at $APP.failed" >&2
else
  echo "no previous bundle to restore" >&2
fi
exit 1
