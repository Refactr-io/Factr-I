import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import path from 'node:path'
import { test } from 'vitest'

import { bundledPython, parseArgs, smokeEnv } from './verify-packaged-app.mjs'

const script = path.join(import.meta.dirname, 'verify-packaged-app.mjs')

test('without an app path the script refuses and runs nothing', () => {
  const run = spawnSync(process.execPath, [script], { encoding: 'utf8' })
  assert.equal(run.status, 2)
  assert.match(run.stderr, /usage:/)
  assert.ok(parseArgs([]).error)
  assert.match(parseArgs(['/tmp/not-an-app']).error, /not an \.app/)
})

test('the smoke runs the bundled interpreter with bytecode writes off under a temp HOME', () => {
  const app = '/Volumes/Some Disk/Factr-I.app'
  const { python, pythonPath } = bundledPython(app)
  assert.equal(python, `${app}/Contents/Resources/backend-python/runtime/bin/python3.12`)
  assert.equal(pythonPath.split(path.delimiter).length, 2)
  const env = smokeEnv(app, '/tmp/h')
  assert.equal(env.PYTHONDONTWRITEBYTECODE, '1')
  assert.equal(env.HOME, '/tmp/h')
  assert.ok(env.FACTR_CONFIG_HOME.startsWith('/tmp/h'))
  assert.equal(env.PYTHONPATH, pythonPath)
})
