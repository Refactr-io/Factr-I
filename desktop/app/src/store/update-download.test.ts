import { afterEach, describe, expect, it, vi } from 'vitest'

import { $autoDownload, resetUpdateDownloadForTests, setAutoDownload } from './update-download'

describe('update download setting', () => {
  afterEach(() => {
    resetUpdateDownloadForTests()
    vi.unstubAllGlobals()
  })

  it('is ON by default', () => {
    expect($autoDownload.get()).toBe(true)
  })

  it('persists through the desktop bridge and adopts the saved value', async () => {
    const set = vi.fn(async (enabled: boolean) => enabled)

    vi.stubGlobal('window', { factrDesktop: { updates: { setAutoDownload: set } } })
    await setAutoDownload(false)

    expect(set).toHaveBeenCalledWith(false)
    expect($autoDownload.get()).toBe(false)
  })

  it('keeps the chosen value when the bridge is missing or fails', async () => {
    vi.stubGlobal('window', { factrDesktop: { updates: { setAutoDownload: vi.fn().mockRejectedValue(new Error('x')) } } })
    await setAutoDownload(false)

    expect($autoDownload.get()).toBe(false)
  })
})
