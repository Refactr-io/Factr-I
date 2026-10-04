import assert from 'node:assert/strict'

import { test } from 'vitest'

import { guestOnboardingEnabled, skipIntroEnabled } from './guest-onboarding'
import { buildSpawnCommand } from './remote-lifecycle'

test('skipIntroEnabled: exactly "1" in env or --skip-intro on argv skips the first-run film', () => {
  assert.equal(skipIntroEnabled([], { FACTR_SKIP_INTRO: '1' }), true)
  assert.equal(skipIntroEnabled(['electron', '.', '--skip-intro'], {}), true)

  assert.equal(skipIntroEnabled([], {}), false)
  assert.equal(skipIntroEnabled([], { FACTR_SKIP_INTRO: 'true' }), false)
})

test('guestOnboardingEnabled: exactly "1" in env or --guest-onboarding on argv turns guest onboarding on', () => {
  assert.equal(guestOnboardingEnabled([], { FACTR_GUEST_ONBOARDING: '1' }), true)
  assert.equal(guestOnboardingEnabled(['electron', '.', '--guest-onboarding'], {}), true)

  assert.equal(guestOnboardingEnabled([], {}), false)
  assert.equal(guestOnboardingEnabled([], { FACTR_GUEST_ONBOARDING: 'true' }), false)
  assert.equal(guestOnboardingEnabled([], { FACTR_GUEST_ONBOARDING: '0' }), false)
  assert.equal(guestOnboardingEnabled(['electron', '.', '--local'], { FACTR_GUEST_ONBOARDING: '' }), false)
})

test('remote SSH spawn command no longer carries a guest-onboarding variable (no backend reads it)', () => {
  const cmd = buildSpawnCommand('/x/factr', 'work', { logPath: '~/.factr/log' })
  assert.match(cmd, /exec env FACTR_DESKTOP=1 /)
  assert.doesNotMatch(cmd, /FACTR_GUEST_ONBOARDING/)
})
