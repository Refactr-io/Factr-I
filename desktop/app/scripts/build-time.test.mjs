import assert from 'node:assert/strict'
import { test } from 'vitest'

import { buildTimeIso } from './utils.mjs'

test('SOURCE_DATE_EPOCH pins the packaged build time; without it the build time is now', () => {
  assert.equal(buildTimeIso({ SOURCE_DATE_EPOCH: '1791648000' }), '2026-10-10T16:00:00.000Z')
  const now = () => new Date('2026-01-02T03:04:05.000Z')
  assert.equal(buildTimeIso({}, now), '2026-01-02T03:04:05.000Z')
  assert.equal(buildTimeIso({ SOURCE_DATE_EPOCH: 'not-a-number' }, now), '2026-01-02T03:04:05.000Z')
})
