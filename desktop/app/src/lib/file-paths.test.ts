import { afterEach, describe, expect, it, vi } from 'vitest'

import { findPathRefs, resolveExistingFile } from './file-paths'

const texts = (s: string) => findPathRefs(s).map(h => h.text)

describe('findPathRefs', () => {
  it('finds absolute, relative, bare and :line paths', () => {
    expect(texts('see /Users/x/proj/index.html, src/app.ts:42 and report.pdf.')).toEqual([
      '/Users/x/proj/index.html',
      'src/app.ts:42',
      'report.pdf'
    ])
  })

  it('ignores urls, versions, prose words', () => {
    expect(texts('v1.2.3 at https://example.com/a/b and plain words')).toEqual([])
  })
})

describe('resolveExistingFile', () => {
  afterEach(() => vi.unstubAllGlobals())

  it('resolves relative paths against cwd and drops non-existent', async () => {
    const normalizePreviewTarget = vi.fn(async (p: string) =>
      p === 'src/real.ts' ? { kind: 'file', label: 'real.ts', path: '/w/src/real.ts', source: p, url: '' } : null
    )

    vi.stubGlobal('window', { factrDesktop: { normalizePreviewTarget } })
    expect((await resolveExistingFile('src/real.ts', '/w'))?.path).toBe('/w/src/real.ts')
    expect(normalizePreviewTarget).toHaveBeenCalledWith('src/real.ts', '/w')
    expect(await resolveExistingFile('a/b', '/w')).toBeNull()
  })
})
