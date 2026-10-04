import assert from 'node:assert/strict'

import { test } from 'vitest'

import { hasWindowsPathPrefix, isFactrOwnedVenvDaemon } from './venv-holder-select'

const SCRIPTS = 'C:\\Factr\\venv\\Scripts'

test('matches the hindsight daemon shim (exe under venv Scripts + hindsight cmdline)', () => {
  assert.equal(
    isFactrOwnedVenvDaemon(
      'C:\\Factr\\venv\\Scripts\\pythonw.exe',
      'C:\\Factr\\venv\\Scripts\\pythonw.exe -m hindsight_api.main --daemon --idle-timeout 300 --port 9177',
      SCRIPTS
    ),
    true
  )
})

test('Windows path prefix match is ordinal case-insensitive', () => {
  assert.equal(
    isFactrOwnedVenvDaemon(
      'c:\\factr\\venv\\scripts\\python.exe',
      'python.exe -m hindsight_api.main --daemon',
      'C:\\Factr\\venv\\Scripts'
    ),
    true
  )
})

test('excludes external venv holders that are not the hindsight daemon', () => {
  // a user terminal running the factr CLI from the venv — must NOT be killed
  assert.equal(isFactrOwnedVenvDaemon('C:\\Factr\\venv\\Scripts\\factr.exe', 'factr chat -q "hi"', SCRIPTS), false)
  // an unrelated python script using the venv interpreter
  assert.equal(
    isFactrOwnedVenvDaemon('C:\\Factr\\venv\\Scripts\\python.exe', 'python C:\\tools\\import.py', SCRIPTS),
    false
  )
})

test('excludes exes outside the venv even when the cmdline mentions hindsight', () => {
  assert.equal(
    isFactrOwnedVenvDaemon('C:\\Other\\pythonw.exe', 'pythonw -m hindsight_api.main --daemon', SCRIPTS),
    false
  )
})

test('prefix boundary: sibling dirs (ScriptsX) do not match', () => {
  assert.equal(hasWindowsPathPrefix('C:\\Factr\\venv\\ScriptsX\\python.exe', SCRIPTS), false)
  assert.equal(hasWindowsPathPrefix('C:\\Factr\\venv\\Scripts\\python.exe', SCRIPTS), true)
})

test('null/undefined fields never match', () => {
  assert.equal(isFactrOwnedVenvDaemon(null, 'x', SCRIPTS), false)
  assert.equal(isFactrOwnedVenvDaemon('C:\\Factr\\venv\\Scripts\\pythonw.exe', null, SCRIPTS), false)
  assert.equal(isFactrOwnedVenvDaemon(undefined, undefined, SCRIPTS), false)
})
