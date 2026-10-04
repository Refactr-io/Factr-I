import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { test } from 'vitest'

import { buildManifest, probeEngine, writeFactrManifest } from './factr-manifest.mjs'

test('the manifest ties engine, factr source and python together under one id', () => {
  const stage = { factrSha: 'a'.repeat(40), factrDirty: false, pythonVersion: '3.12.12' }
  const a = buildManifest({ engine: { version: '0.88.0', sha: '06ddb7d' }, stage, desktopVersion: '0.17.6', builtAt: 't' })
  assert.deepEqual([a.engine.sha, a.factr.sha, a.python.version], ['06ddb7d', 'a'.repeat(40), '3.12.12'])
  const b = buildManifest({ engine: { version: '0.88.0', sha: '1234567' }, stage, desktopVersion: '0.17.6', builtAt: 't' })
  assert.notEqual(a.id, b.id, 'any part changing changes the build id')
})

test('probeEngine reads the binary itself and gives up quietly when it cannot run', () => {
  assert.deepEqual(probeEngine('x', () => '{"version":"0.88.0","sha":"06ddb7d"}'), { version: '0.88.0', sha: '06ddb7d' })
  assert.deepEqual(probeEngine('x', () => '{"version":"0.88.0","sha":"06ddb7d","db_schema":4}'), { version: '0.88.0', sha: '06ddb7d', dbSchema: 4 })
  assert.equal(probeEngine('x', () => { throw new Error('bad CPU type') }), null)
  assert.equal(probeEngine('x', () => 'not json'), null)
})

test('writeFactrManifest needs the staged runtime and falls back to the engine Cargo version', () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'sov-manifest-'))
  const engineRoot = path.join(dir, 'engine')
  fs.mkdirSync(engineRoot)
  fs.writeFileSync(path.join(engineRoot, 'Cargo.toml'), '[package]\nname = "x"\nversion = "0.88.0"\n')
  const stagePath = path.join(dir, 'stage.json')
  const args = { binary: path.join(dir, 'factr'), canRun: false, engineRoot, stagePath, outDir: dir, desktopVersion: '1' }
  assert.throws(() => writeFactrManifest(args), /stage:backend-python/)
  fs.writeFileSync(stagePath, JSON.stringify({ factrSha: 'b'.repeat(40), pythonVersion: '3.12.12' }))
  fs.writeFileSync(args.binary, 'engine')
  const written = writeFactrManifest(args)
  assert.deepEqual(JSON.parse(fs.readFileSync(path.join(dir, 'manifest.json'), 'utf8')), written)
  assert.deepEqual(written.engine, { version: '0.88.0', sha: null, sha256: createHash('sha256').update('engine').digest('hex') })
  fs.rmSync(dir, { recursive: true, force: true })
})
