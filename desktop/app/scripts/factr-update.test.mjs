import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { test as baseTest } from 'vitest'

// The update script relies on macOS-only tools (ditto).
const test = baseTest.skipIf(process.platform !== 'darwin')

const script = path.join(import.meta.dirname, 'factr-update.sh')

function fakeApp(dir, id, dbSchema) {
  const res = path.join(dir, 'Contents/Resources')
  fs.mkdirSync(path.join(res, 'backend-python/runtime'), { recursive: true })
  fs.mkdirSync(path.join(res, 'factr'), { recursive: true })
  fs.writeFileSync(path.join(res, 'factr/factr'), '')
  fs.writeFileSync(path.join(res, 'factr/manifest.json'), JSON.stringify({ id, ...(dbSchema ? { engine: { dbSchema } } : {}) }, null, 2))
  return dir
}

// A stand-in for `open`: the launched app reports healthy iff its own manifest id is in HEALTHY_IDS.
function fakeOpen(dir, userData) {
  const file = path.join(dir, 'open.sh')
  fs.writeFileSync(
    file,
    `#!/bin/sh\nid=$(sed -n 's/.*"id": *"\\([^"]*\\)".*/\\1/p' "$1/Contents/Resources/factr/manifest.json")\n` +
      `case " $HEALTHY_IDS " in *" $id "*) printf '{"id":"%s"}' "$id" > "${userData}/launch-ok.json";; esac\n`
  )
  fs.chmodSync(file, 0o755)
  return file
}

function run(healthy, migrate = false, attempts = 1, dbName = 'factr', interrupted = false) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'sov-update-'))
  const userData = path.join(dir, 'userdata')
  fs.mkdirSync(userData)
  fs.writeFileSync(path.join(userData, 'keep.txt'), 'user data')
  const app = fakeApp(path.join(dir, 'Factr.app'), 'old')
  const next = fakeApp(path.join(dir, 'next.app'), 'new')
  // an earlier update died after moving the installed app aside and before putting the new one in
  if (interrupted) fs.renameSync(app, `${app}.previous`)
  const dbDir = path.join(dir, dbName)
  fs.mkdirSync(dbDir)
  fs.writeFileSync(path.join(dbDir, 'factr.db'), 'v1')
  const stale = path.join(dbDir, 'factr.db.pre-v1.bak')
  fs.writeFileSync(stale, 'stale')
  fs.utimesSync(stale, 1, 1)
  let open = fakeOpen(dir, userData)
  if (migrate) {
    // the new engine backs up, migrates, and leaves -wal/-shm behind before failing its health check
    open = path.join(dir, 'open-migrate.sh')
    fs.writeFileSync(open, `#!/bin/sh\ngrep -q '"new"' "$1/Contents/Resources/factr/manifest.json" || exit 0\nsleep 1\n[ -e "${dbDir}/factr.db.pre-v2.bak" ] || printf pre-update > "${dbDir}/factr.db.pre-v2.bak"\nprintf v2 > "${dbDir}/factr.db"\nprintf w > "${dbDir}/factr.db-wal"\nprintf s > "${dbDir}/factr.db-shm"\n`)
    fs.chmodSync(open, 0o755)
  }
  let result
  for (let n = 0; n < attempts; n++) result = spawnSync('sh', [script, next, app], {
    encoding: 'utf8',
    env: {
      ...process.env,
      HEALTHY_IDS: healthy,
      FACTR_UPDATE_USERDATA: userData,
      FACTR_UPDATE_OPEN: open,
      FACTR_UPDATE_DBDIR: dbDir,
      FACTR_UPDATE_WAIT: '2',
      FACTR_UPDATE_SKIP_RUNNING_CHECK: '1'
    }
  })
  const idOf = p => JSON.parse(fs.readFileSync(path.join(p, 'Contents/Resources/factr/manifest.json'), 'utf8')).id
  return { dir, app, userData, dbDir, result, idOf }
}

test('a healthy new build replaces all three parts and keeps the previous bundle', () => {
  const { dir, app, userData, result, idOf } = run('new old')
  assert.equal(result.status, 0, result.stderr)
  assert.equal(idOf(app), 'new')
  assert.equal(idOf(`${app}.previous`), 'old')
  assert.equal(fs.readFileSync(path.join(userData, 'keep.txt'), 'utf8'), 'user data')
  fs.rmSync(dir, { recursive: true, force: true })
})

test('a build that never reports healthy is rolled back and user data is untouched', () => {
  const { dir, app, userData, result, idOf } = run('old')
  assert.equal(result.status, 1)
  assert.equal(idOf(app), 'old')
  assert.equal(idOf(`${app}.failed`), 'new')
  assert.equal(fs.readFileSync(path.join(userData, 'keep.txt'), 'utf8'), 'user data')
  fs.rmSync(dir, { recursive: true, force: true })
})

test('a rollback restores the factr.db backup taken during the update and nothing older', () => {
  const { dir, dbDir, userData, result } = run('', true)
  assert.equal(result.status, 1)
  assert.equal(fs.readFileSync(path.join(dbDir, 'factr.db'), 'utf8'), 'pre-update')
  assert.ok(!fs.existsSync(path.join(dbDir, 'factr.db-wal')) && !fs.existsSync(path.join(dbDir, 'factr.db-shm')))
  assert.equal(fs.readFileSync(path.join(userData, 'keep.txt'), 'utf8'), 'user data')
  fs.rmSync(dir, { recursive: true, force: true })
})

test('an update clears stale pre-migration backups so the new engine writes a fresh one', () => {
  const { dir, dbDir, result } = run('new old')
  assert.equal(result.status, 0, result.stderr)
  assert.ok(!fs.existsSync(path.join(dbDir, 'factr.db.pre-v1.bak')))
  assert.equal(fs.readFileSync(path.join(dbDir, 'factr.db'), 'utf8'), 'v1')
  fs.rmSync(dir, { recursive: true, force: true })
})

test('a second failed attempt still finds a fresh backup (the used one is removed)', () => {
  const { dir, dbDir, result } = run('', true, 2)
  assert.equal(result.status, 1)
  assert.equal(fs.readFileSync(path.join(dbDir, 'factr.db'), 'utf8'), 'pre-update')
  assert.ok(!fs.existsSync(path.join(dbDir, 'factr.db.pre-v2.bak')))
  fs.rmSync(dir, { recursive: true, force: true })
}, 30000)

test('a rollback finds and restores the backup when the database directory has spaces', () => {
  const { dir, dbDir, result } = run('', true, 1, 'my factr home')
  assert.equal(result.status, 1)
  assert.equal(fs.readFileSync(path.join(dbDir, 'factr.db'), 'utf8'), 'pre-update')
  fs.rmSync(dir, { recursive: true, force: true })
})

test('a bundle whose engine hash does not match its manifest is refused before anything is touched', () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'sov-update-'))
  const app = fakeApp(path.join(dir, 'Factr.app'), 'old')
  const next = fakeApp(path.join(dir, 'next.app'), 'new')
  const manifest = path.join(next, 'Contents/Resources/factr/manifest.json')
  const env = { ...process.env, FACTR_UPDATE_SKIP_RUNNING_CHECK: '1', FACTR_UPDATE_OPEN: 'true', FACTR_UPDATE_WAIT: '1', FACTR_UPDATE_USERDATA: dir, FACTR_UPDATE_DBDIR: dir }
  fs.writeFileSync(manifest, JSON.stringify({ id: 'new', engine: { sha256: 'a'.repeat(64) } }))
  const bad = spawnSync('sh', [script, next, app], { encoding: 'utf8', env })
  assert.equal(bad.status, 2)
  assert.match(bad.stderr, /does not match the manifest/)
  assert.ok(!fs.existsSync(`${app}.previous`))
  // the right hash (of the empty stand-in binary) gets past the check
  fs.writeFileSync(manifest, JSON.stringify({ id: 'new', engine: { sha256: 'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855' } }))
  const good = spawnSync('sh', [script, next, app], { encoding: 'utf8', env })
  assert.notEqual(good.status, 2, good.stderr)
  fs.rmSync(dir, { recursive: true, force: true })
})

test('an interrupted earlier swap is repaired from .previous, not deleted', () => {
  const ok = run('new old', false, 1, 'factr', true)
  assert.equal(ok.result.status, 0, ok.result.stderr)
  assert.match(ok.result.stderr, /earlier update was interrupted/)
  assert.equal(ok.idOf(ok.app), 'new')
  assert.equal(ok.idOf(`${ok.app}.previous`), 'old')
  fs.rmSync(ok.dir, { recursive: true, force: true })
  // and if the new build then fails, the rollback still has the old bundle to return to
  const bad = run('', false, 1, 'factr', true)
  assert.equal(bad.result.status, 1)
  assert.equal(bad.idOf(bad.app), 'old')
  fs.rmSync(bad.dir, { recursive: true, force: true })
})

test('the health wait is configurable and doubles when the new build migrates factr.db further', () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'sov-update-'))
  const userData = path.join(dir, 'userdata')
  fs.mkdirSync(userData)
  const attempt = (installedSchema, newSchema, waitVar) => {
    fs.rmSync(path.join(dir, 'Factr.app'), { recursive: true, force: true })
    fs.rmSync(path.join(dir, 'Factr.app.previous'), { recursive: true, force: true })
    fs.rmSync(path.join(dir, 'Factr.app.failed'), { recursive: true, force: true })
    const app = fakeApp(path.join(dir, 'Factr.app'), 'old', installedSchema)
    fs.rmSync(path.join(dir, 'next.app'), { recursive: true, force: true })
    const next = fakeApp(path.join(dir, 'next.app'), 'new', newSchema)
    const env = { ...process.env, FACTR_UPDATE_SKIP_RUNNING_CHECK: '1', FACTR_UPDATE_OPEN: 'true', FACTR_UPDATE_USERDATA: userData, FACTR_UPDATE_DBDIR: dir, FACTR_UPDATE_WAIT: '', [waitVar]: '1' }
    return spawnSync('sh', [script, next, app], { encoding: 'utf8', env }).stderr
  }
  assert.match(attempt(1, 1, 'FACTR_UPDATE_WAIT_S'), /within 1s/)
  assert.match(attempt(1, 2, 'FACTR_UPDATE_WAIT_S'), /within 2s/)
  assert.match(attempt(undefined, 2, 'FACTR_UPDATE_WAIT_S'), /within 1s/, 'unknown installed schema: no automatic extension')
  assert.match(attempt(2, 2, 'FACTR_UPDATE_WAIT'), /within 1s/, 'the old variable name still works')
  fs.rmSync(dir, { recursive: true, force: true })
}, 30000)

test('a rollback with no migration leaves factr.db alone', () => {
  const { dir, dbDir, result } = run('old')
  assert.equal(result.status, 1)
  assert.equal(fs.readFileSync(path.join(dbDir, 'factr.db'), 'utf8'), 'v1')
  fs.rmSync(dir, { recursive: true, force: true })
})

test('a bundle missing a part is refused before anything is touched', () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'sov-update-'))
  const app = fakeApp(path.join(dir, 'Factr.app'), 'old')
  const broken = fakeApp(path.join(dir, 'next.app'), 'new')
  fs.rmSync(path.join(broken, 'Contents/Resources/backend-python'), { recursive: true })
  const result = spawnSync('sh', [script, broken, app], { encoding: 'utf8', env: { ...process.env, FACTR_UPDATE_SKIP_RUNNING_CHECK: '1' } })
  assert.equal(result.status, 2)
  assert.ok(fs.existsSync(path.join(app, 'Contents/Resources/factr/manifest.json')))
  assert.ok(!fs.existsSync(`${app}.previous`))
  fs.rmSync(dir, { recursive: true, force: true })
})
