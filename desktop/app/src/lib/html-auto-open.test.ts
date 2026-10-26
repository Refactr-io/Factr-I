import { beforeEach, describe, expect, it, vi } from 'vitest'

import type * as DesktopFs from '@/lib/desktop-fs'
import type * as LocalPreview from '@/lib/local-preview'
import { $previewTabs, closePreviewForSource } from '@/store/preview'

import { autoOpenHtmlFile, isLocalHtmlTarget, resetHtmlAutoOpen } from './html-auto-open'
import { localPreviewTarget } from './local-preview'

const remote = vi.hoisted(() => ({ on: false }))

vi.mock('@/lib/desktop-fs', async importOriginal => ({
  ...(await importOriginal<typeof DesktopFs>()),
  isDesktopFsRemoteMode: () => remote.on
}))

vi.mock('@/lib/local-preview', async importOriginal => {
  const original = await importOriginal<typeof LocalPreview>()

  return {
    ...original,
    normalizeOrLocalPreviewTarget: async (raw: string, cwd?: string) => original.localPreviewTarget(raw, cwd)
  }
})

describe('isLocalHtmlTarget', () => {
  it('accepts paths and file URLs of html files, and nothing else', () => {
    expect(isLocalHtmlTarget('/work/site/index.html')).toBe(true)
    expect(isLocalHtmlTarget('page.HTM')).toBe(true)
    expect(isLocalHtmlTarget('file:///work/site/a.html#top')).toBe(true)
    expect(isLocalHtmlTarget('/work/report.pdf')).toBe(false)
    expect(isLocalHtmlTarget('/work/app.js')).toBe(false)
    expect(isLocalHtmlTarget('https://example.com/index.html')).toBe(false)
    expect(isLocalHtmlTarget('data:text/html;base64,AAAA')).toBe(false)
  })
})

describe('autoOpenHtmlFile', () => {
  beforeEach(() => {
    remote.on = false
    $previewTabs.set([])
    resetHtmlAutoOpen()
  })

  it('opens a new page in the pane once, then leaves a page the user closed alone', async () => {
    expect(await autoOpenHtmlFile('s1', '/work/site/index.html', '/work')).toBe('opened')
    expect($previewTabs.get()).toHaveLength(1)
    expect($previewTabs.get()[0]?.target).toMatchObject({ kind: 'file', previewKind: 'html', renderMode: 'preview' })

    // An edit while the tab is open needs nothing: the file tab reloads itself from disk.
    expect(await autoOpenHtmlFile('s1', '/work/site/index.html', '/work')).toBe('already-open')
    expect($previewTabs.get()).toHaveLength(1)

    closePreviewForSource($previewTabs.get()[0]!.target.source)
    expect($previewTabs.get()).toHaveLength(0)
    expect(await autoOpenHtmlFile('s1', '/work/site/index.html', '/work')).toBe('skipped')
    expect($previewTabs.get()).toHaveLength(0)
  })

  it('treats each conversation and each file on its own', async () => {
    expect(await autoOpenHtmlFile('s1', 'a.html', '/work')).toBe('opened')
    expect(await autoOpenHtmlFile('s1', 'b.html', '/work')).toBe('opened')
    expect($previewTabs.get()).toHaveLength(2)
    closePreviewForSource($previewTabs.get()[0]!.target.source)
    expect(await autoOpenHtmlFile('s2', 'a.html', '/work')).toBe('opened')
  })

  it('leaves other files and remote files to their existing behaviour', async () => {
    expect(await autoOpenHtmlFile('s1', '/work/report.pdf')).toBe('unsupported')
    expect(await autoOpenHtmlFile('s1', '/work/notes.md')).toBe('unsupported')
    remote.on = true
    expect(await autoOpenHtmlFile('s1', '/work/site/index.html')).toBe('unsupported')
    expect($previewTabs.get()).toHaveLength(0)
    expect(localPreviewTarget('/work/report.pdf')?.previewKind).toBe('pdf')
  })
})
