import { describe, expect, it } from 'vitest'

import { botsHidden, HIDDEN_BUNDLED_PLUGINS } from './hidden-features'

describe('hidden features', () => {
  it('ships Bot Mode so the plugin registers', () => {
    expect(HIDDEN_BUNDLED_PLUGINS.has('factr-bots')).toBe(false)
    expect(botsHidden).toBe(false)
  })
})
