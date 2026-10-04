import { describe, expect, it } from 'vitest'

import { buildFirstTaskRunbook, pluginsRunbook } from '@/components/onboarding-chat/setup-profile'
import { DEFAULT_ANSWERS } from '@/store/onboarding-answers'

describe('plugins in the handoff runbook', () => {
  it('adds nothing when no plugin was picked', () => {
    expect(pluginsRunbook(DEFAULT_ANSWERS)).toBe('')
    expect(buildFirstTaskRunbook('Organize my work', DEFAULT_ANSWERS)).not.toContain('PLUGINS FROM ONBOARDING')
  })
})
