import { describe, expect, it } from 'vitest'

import { previewWatchWantsRecursive, shouldReloadPreviewFor } from './preview-watch'

describe('preview file watch', () => {
  it('reloads an HTML page when a stylesheet, script or image beside it is written after it', () => {
    for (const asset of ['style.css', 'app.js', 'logo.svg', 'assets/photo.png', 'data.json']) {
      expect(shouldReloadPreviewFor('index.html', asset)).toBe(true)
    }
  })

  it('still reloads for the page itself and for an unnamed change', () => {
    expect(shouldReloadPreviewFor('index.html', 'index.html')).toBe(true)
    expect(shouldReloadPreviewFor('index.html', '')).toBe(true)
  })

  it('ignores unrelated files next to the page', () => {
    for (const other of ['notes.txt', 'run.log', '.DS_Store', 'archive.zip']) {
      expect(shouldReloadPreviewFor('index.html', other)).toBe(false)
    }
  })

  it('leaves a non-HTML preview watching only its own file', () => {
    expect(shouldReloadPreviewFor('report.md', 'style.css')).toBe(false)
    expect(shouldReloadPreviewFor('report.md', 'report.md')).toBe(true)
    expect(previewWatchWantsRecursive('report.md')).toBe(false)
    expect(previewWatchWantsRecursive('index.html')).toBe(true)
  })
})
