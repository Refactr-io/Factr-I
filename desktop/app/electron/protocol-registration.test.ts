import assert from 'node:assert/strict'

import { test } from 'vitest'

import { shouldRegisterProtocolClient } from './protocol-registration'

test('the app claims factr:// unless FACTR_DISABLE_PROTOCOL_REGISTRATION=1', () => {
  assert.equal(shouldRegisterProtocolClient({}), true)
  assert.equal(shouldRegisterProtocolClient({ FACTR_DISABLE_PROTOCOL_REGISTRATION: '' }), true)
  assert.equal(shouldRegisterProtocolClient({ FACTR_DISABLE_PROTOCOL_REGISTRATION: '0' }), true)
  assert.equal(shouldRegisterProtocolClient({ FACTR_DISABLE_PROTOCOL_REGISTRATION: '1' }), false)
})
