import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'

import { test } from 'vitest'

const main = fs.readFileSync(path.join(__dirname, 'main.ts'), 'utf8')

test('userData is Factr-I and is never renamed into or out of the engine config folder', () => {
  assert.match(main, /path\.join\(app\.getPath\('appData'\), 'Factr-I'\)/)
  // macOS/Windows are case-insensitive: "Factr" vs the engine's "factr" would alias.
  assert.doesNotMatch(main, /path\.join\(appData, 'Factr'\)/)
  assert.doesNotMatch(main, /renameSync\(legacy/)
  assert.notEqual('Factr-I'.toLowerCase(), 'factr')
})

test('the default config home is ~/.factr on every platform, never LOCALAPPDATA', () => {
  const start = main.indexOf('function resolveFactrHome')
  const body = main.slice(start, main.indexOf('const FACTR_CONFIG_HOME', start))
  assert.doesNotMatch(body, /LOCALAPPDATA/)
  assert.match(body, /'\.factr'/)
})
