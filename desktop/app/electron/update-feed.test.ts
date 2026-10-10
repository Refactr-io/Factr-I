import { describe, expect, it, vi } from 'vitest'

import { compareSemver, createUpdateFeed, parseSemver, RELEASES_PAGE_URL, serverWaitMs } from './update-feed'

const release = (over: Record<string, unknown> = {}) => ({
  tag_name: 'v0.2.0',
  html_url: 'https://github.com/Refactr-io/Factr-I/releases/tag/v0.2.0',
  body: 'Notes',
  draft: false,
  prerelease: false,
  ...over
})

const reply = (status: number, body: unknown = {}, etag: string | null = null, extra: Record<string, string> = {}) => ({
  status,
  headers: { get: (n: string) => (n.toLowerCase() === 'etag' ? etag : (extra[n.toLowerCase()] ?? null)) },
  json: async () => body
})

function setup(version = '0.1.0', fetchImpl = vi.fn()) {
  let t = 1_000_000
  const openExternal = vi.fn(async () => undefined)
  const feed = createUpdateFeed({ fetch: fetchImpl, getVersion: () => version, now: () => t, openExternal })

  return { feed, fetchImpl, openExternal, advance: (ms: number) => (t += ms) }
}

describe('semver', () => {
  it('parses and orders', () => {
    expect(parseSemver('v1.2.3')).toEqual({ core: [1, 2, 3], pre: [] })
    expect(parseSemver('nightly')).toBeNull()
    expect(compareSemver(parseSemver('1.10.0')!, parseSemver('1.9.0')!)).toBeGreaterThan(0)
    expect(compareSemver(parseSemver('1.0.0-beta.1')!, parseSemver('1.0.0')!)).toBeLessThan(0)
    expect(compareSemver(parseSemver('1.0.0-beta.2')!, parseSemver('1.0.0-beta.10')!)).toBeLessThan(0)
  })
})

describe('update feed check', () => {
  it('reports a newer release', async () => {
    const { feed } = setup('0.1.0', vi.fn(async () => reply(200, release(), 'W/"a"')))
    const s = await feed.check()

    expect(s).toMatchObject({
      supported: true,
      updateAvailable: true,
      currentVersion: '0.1.0',
      latestVersion: '0.2.0',
      releaseUrl: 'https://github.com/Refactr-io/Factr-I/releases/tag/v0.2.0',
      notes: 'Notes'
    })
  })

  it('reports none for same or older', async () => {
    for (const tag of ['v0.1.0', 'v0.0.9']) {
      const { feed } = setup('0.1.0', vi.fn(async () => reply(200, release({ tag_name: tag }))))

      expect((await feed.check()).updateAvailable).toBe(false)
    }
  })

  it('treats 404 as no release, without error', async () => {
    const { feed } = setup('0.1.0', vi.fn(async () => reply(404)))
    const s = await feed.check()

    expect(s.updateAvailable).toBe(false)
    expect(s.error).toBeUndefined()
  })

  it('ignores prerelease, draft and invalid tags', async () => {
    for (const body of [release({ prerelease: true }), release({ draft: true }), release({ tag_name: 'nightly' })]) {
      const { feed } = setup('0.1.0', vi.fn(async () => reply(200, body)))
      const s = await feed.check()

      expect(s.updateAvailable).toBe(false)
      expect(s.error).toBeUndefined()
    }
  })

  it('surfaces rate limit and offline as a check error, keeping the last good result', async () => {
    const f = vi.fn()
    f.mockResolvedValueOnce(reply(200, release(), 'e1')).mockResolvedValueOnce(reply(403)).mockRejectedValueOnce(new Error('offline'))
    const { feed, advance } = setup('0.1.0', f)

    await feed.check()
    advance(7 * 3600_000)
    const limited = await feed.check()

    expect(limited).toMatchObject({ error: 'check-failed', updateAvailable: true })
    advance(16 * 60_000)
    const offline = await feed.check()

    expect(offline).toMatchObject({ error: 'check-failed', message: 'offline' })
  })

  it('offline with no prior result is an error with no update', async () => {
    const { feed } = setup('0.1.0', vi.fn(async () => Promise.reject(new Error('offline'))))

    expect(await feed.check()).toMatchObject({ error: 'check-failed', updateAvailable: false })
  })

  it('honours the 6 h interval, then revalidates with If-None-Match', async () => {
    const f = vi.fn()
    f.mockResolvedValueOnce(reply(200, release(), 'etag-1')).mockResolvedValueOnce(reply(304))
    const { feed, advance } = setup('0.1.0', f)

    await feed.check()
    advance(60 * 60 * 1000)
    await feed.check()
    expect(f).toHaveBeenCalledTimes(1)

    advance(6 * 60 * 60 * 1000)
    const s = await feed.check()

    expect(f).toHaveBeenCalledTimes(2)
    expect(f.mock.calls[1][1].headers['If-None-Match']).toBe('etag-1')
    expect(s.updateAvailable).toBe(true)
  })

  it('force bypasses the interval but not the 30 s floor', async () => {
    const f = vi.fn(async () => reply(200, release(), 'e'))
    const { feed, advance } = setup('0.1.0', f)

    await feed.check()
    await feed.check({ force: true })
    expect(f).toHaveBeenCalledTimes(1)
    advance(31_000)
    await feed.check({ force: true })
    expect(f).toHaveBeenCalledTimes(2)
  })
})

describe('update feed backoff', () => {
  const headers = (h: Record<string, string>) => ({ get: (n: string) => h[n.toLowerCase()] ?? null })

  it('reads Retry-After seconds or date, and the rate-limit reset when nothing is left', () => {
    const now = Date.parse('2026-10-10T12:00:00Z')

    expect(serverWaitMs(headers({ 'retry-after': '120' }), now)).toBe(120_000)
    expect(serverWaitMs(headers({ 'retry-after': 'Sat, 10 Oct 2026 12:05:00 GMT' }), now)).toBe(300_000)
    expect(serverWaitMs(headers({ 'x-ratelimit-remaining': '0', 'x-ratelimit-reset': String(now / 1000 + 600) }), now)).toBe(600_000)
    expect(serverWaitMs(headers({ 'x-ratelimit-remaining': '12', 'x-ratelimit-reset': String(now / 1000 + 600) }), now)).toBe(0)
    expect(serverWaitMs(headers({ 'retry-after': '999999999' }), now)).toBe(24 * 3600_000)
    expect(serverWaitMs(headers({ 'retry-after': 'soon' }), now)).toBe(0)
    expect(serverWaitMs(headers({}), now)).toBe(0)
  })

  it('does not ask again right after a failure, then retries after the backoff', async () => {
    const f = vi.fn()
    f.mockRejectedValueOnce(new Error('offline')).mockResolvedValueOnce(reply(200, release(), 'e'))
    const { feed, advance } = setup('0.1.0', f)

    expect(await feed.check()).toMatchObject({ error: 'check-failed' })
    expect(await feed.check()).toMatchObject({ error: 'check-failed' })
    advance(60_000)
    expect(await feed.check()).toMatchObject({ error: 'check-failed' })
    expect(f).toHaveBeenCalledTimes(1)

    // A forced check (the user pressing "Check now") may retry after the 30 s floor.
    expect(await feed.check({ force: true })).toMatchObject({ updateAvailable: true })
    expect(f).toHaveBeenCalledTimes(2)
    expect((await feed.check()).error).toBeUndefined()
  })

  it('honours the server wait even for forced checks', async () => {
    const f = vi.fn()
    f.mockResolvedValueOnce(reply(429, {}, null, { 'retry-after': '3600' })).mockResolvedValueOnce(reply(200, release()))
    const { feed, advance } = setup('0.1.0', f)

    expect(await feed.check()).toMatchObject({ message: 'GitHub rate limit reached.' })
    advance(30 * 60_000)
    await feed.check({ force: true })
    expect(f).toHaveBeenCalledTimes(1)
    advance(31 * 60_000)
    expect(await feed.check({ force: true })).toMatchObject({ updateAvailable: true })
    expect(f).toHaveBeenCalledTimes(2)
  })
})

describe('update feed apply', () => {
  it('opens the validated release page', async () => {
    const { feed, openExternal } = setup('0.1.0', vi.fn(async () => reply(200, release())))

    await feed.check()
    expect(await feed.apply()).toMatchObject({ ok: true, openedReleasePage: true })
    expect(openExternal).toHaveBeenCalledWith('https://github.com/Refactr-io/Factr-I/releases/tag/v0.2.0')
  })

  it('never opens a foreign html_url', async () => {
    const { feed, openExternal } = setup('0.1.0', vi.fn(async () => reply(200, release({ html_url: 'https://evil.example/x' }))))

    await feed.check()
    await feed.apply()
    expect(openExternal).toHaveBeenCalledWith('https://github.com/Refactr-io/Factr-I/releases/tag/v0.2.0')
  })

  it('falls back to the releases page when nothing is known', async () => {
    const { feed, openExternal } = setup()

    await feed.apply()
    expect(openExternal).toHaveBeenCalledWith(RELEASES_PAGE_URL)
  })
})
