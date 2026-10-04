import { describe, expect, it } from 'vitest'

import type { SessionInfo } from '@/factr'

import { sessionTitle } from './chat-runtime'

const row = (title: null | string, preview: null | string) => ({ preview, title }) as SessionInfo

describe('sessionTitle placeholders', () => {
  it('prefers the first user message over a generic engine title', () => {
    expect(sessionTitle(row('New chat', 'summarise the repo'))).toBe('summarise the repo')
    expect(sessionTitle(row('New chat', null))).toBe('New chat')
    expect(sessionTitle(row('Release notes', 'summarise the repo'))).toBe('Release notes')
  })
})
