import { describe, expect, it } from 'vitest'

import type { Contribution } from '@/contrib/registry'

import { group, split } from '../model'

import { fixedTrackSize, type TrackContext } from './track-model'

const pane = (id: string, placement: string, size: { height?: string; width?: string } = {}) =>
  ({ id, area: 'panes', data: { placement, ...size }, render: () => null }) as unknown as Contribution

const PANES: Record<string, Contribution> = {
  files: pane('files', 'right', { width: '20rem' }),
  review: pane('review', 'right', { width: '20rem' }),
  sessions: pane('sessions', 'left', { width: '16rem' }),
  terminal: pane('terminal', 'bottom', { height: '14rem' }),
  workspace: pane('workspace', 'main')
}

const ctx: TrackContext = { overrides: {}, paneFor: id => PANES[id], paneGone: () => false }

describe('a column across a row', () => {
  it('stays flexible when it holds the chat: a card docked above or below must not hand its width to the column', () => {
    expect(fixedTrackSize(split('column', [group(['workspace']), group(['files'])]), 'row', ctx)).toBeNull()
    expect(fixedTrackSize(split('column', [group(['files']), group(['workspace'])]), 'row', ctx)).toBeNull()
  })

  it('still takes the width of its fixed cards when it holds no chat (the tools column)', () => {
    const tools = split('column', [group(['terminal']), group(['files']), group(['review'])])

    expect(fixedTrackSize(tools, 'row', ctx)).toBe('20rem')
  })
})
