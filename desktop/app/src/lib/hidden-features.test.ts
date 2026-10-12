import { describe, expect, it } from 'vitest'

import { botsHidden, HIDDEN_BUNDLED_PLUGINS } from './hidden-features'

describe('hidden features', () => {
  it('hides Bot Mode so the plugin never registers', () => {
    expect(HIDDEN_BUNDLED_PLUGINS.has('factr-bots')).toBe(true)
    expect(botsHidden).toBe(true)
  })
})
