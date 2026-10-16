import { createHash } from 'node:crypto'

import { describe, expect, it, vi } from 'vitest'

import {
  createUpdateDownloader,
  type DownloaderDeps,
  type DownloadFetchResponse,
  isAllowedDownloadUrl
} from './update-download'
import type { FeedStatus } from './update-feed'

const bytes = (text: string) => new TextEncoder().encode(text)
const sha = (data: Uint8Array) => createHash('sha256').update(data).digest('hex')

const URL_OK = 'https://github.com/Refactr-io/Factr-I/releases/download/v0.0.9/Factr-I-0.0.9-mac-arm64.dmg'

function status(data: Uint8Array, overrides: Partial<FeedStatus['asset'] & object> = {}): FeedStatus {
  return {
    supported: true,
    updateAvailable: true,
    currentVersion: '0.0.8',
    latestVersion: '0.0.9',
    fetchedAt: 0,
    asset: { kind: 'dmg', name: 'Factr-I-0.0.9-mac-arm64.dmg', url: URL_OK, size: data.byteLength, sha256: sha(data), ...overrides }
  }
}

async function* chunks(data: Uint8Array, parts = 3, onChunk?: (i: number) => Promise<void>) {
  const step = Math.ceil(data.byteLength / parts)

  for (let i = 0, n = 0; i < data.byteLength; i += step, n += 1) {
    await onChunk?.(n)

    yield data.slice(i, i + step)
  }
}

const ok = (body: AsyncIterable<Uint8Array>): DownloadFetchResponse => ({ status: 200, headers: { get: () => null }, body })

const redirect = (to: string): DownloadFetchResponse => ({
  status: 302,
  headers: { get: (n: string) => (n.toLowerCase() === 'location' ? to : null) },
  body: null
})

function memoryFs() {
  const files = new Map<string, Uint8Array>()
  const dirs = new Set<string>()

  const fs: DownloaderDeps['fs'] = {
    mkdir: async dir => void dirs.add(dir),
    remove: async target => {
      for (const key of [...files.keys()]) {
        if (key === target || key.startsWith(`${target}/`)) {
          files.delete(key)
        }
      }

      for (const dir of [...dirs]) {
        if (dir === target || dir.startsWith(`${target}/`)) {
          dirs.delete(dir)
        }
      }
    },
    list: async dir => [...dirs].filter(d => d.startsWith(`${dir}/`)).map(d => d.slice(dir.length + 1).split('/')[0]!),
    size: async file => files.get(file)?.byteLength ?? null,
    rename: async (from, to) => {
      files.set(to, files.get(from)!)
      files.delete(from)
    },
    open: async file => {
      files.set(file, new Uint8Array())

      return {
        write: async chunk => {
          const prev = files.get(file)!
          const next = new Uint8Array(prev.byteLength + chunk.byteLength)

          next.set(prev)
          next.set(chunk, prev.byteLength)
          files.set(file, next)
        },
        close: async () => undefined
      }
    },
    read: async function* (file) {
      yield files.get(file) ?? new Uint8Array()
    }
  }

  return { fs, files, dirs }
}

function setup(overrides: Partial<DownloaderDeps> = {}) {
  const mem = memoryFs()
  const states: string[] = []

  const deps: DownloaderDeps = {
    fetch: vi.fn(),
    fs: mem.fs,
    updatesDir: '/ud/updates',
    join: (...parts) => parts.join('/'),
    isEnabled: () => true,
    isOnline: () => true,
    isBusy: () => false,
    onState: s => states.push(s.status),
    ...overrides
  }

  return { dl: createUpdateDownloader(deps), deps, mem, states }
}

const settle = () => new Promise(resolve => setTimeout(resolve, 20))

describe('isAllowedDownloadUrl', () => {
  it('allows https on GitHub hosts only', () => {
    expect(isAllowedDownloadUrl(URL_OK)).toBe(true)
    expect(isAllowedDownloadUrl('https://objects.githubusercontent.com/x')).toBe(true)
    expect(isAllowedDownloadUrl('https://release-assets.githubusercontent.com/x')).toBe(true)
    expect(isAllowedDownloadUrl('http://github.com/x')).toBe(false)
    expect(isAllowedDownloadUrl('https://evil.example/x')).toBe(false)
    expect(isAllowedDownloadUrl('https://github.com.evil.example/x')).toBe(false)
    expect(isAllowedDownloadUrl('not a url')).toBe(false)
  })

  it('allows the dev origin only when given', () => {
    expect(isAllowedDownloadUrl('http://127.0.0.1:9000/a')).toBe(false)
    expect(isAllowedDownloadUrl('http://127.0.0.1:9000/a', 'http://127.0.0.1:9000')).toBe(true)
  })
})

describe('createUpdateDownloader', () => {
  it('downloads, verifies size and sha256, renames and goes ready, dropping other versions first', async () => {
    const data = bytes('installer-bytes-0123456789')
    const { dl, deps, mem, states } = setup()

    await deps.fs.mkdir('/ud/updates/0.0.7')
    mem.files.set('/ud/updates/0.0.7/old.dmg', bytes('old'))
    vi.mocked(deps.fetch).mockResolvedValue(ok(chunks(data)))

    dl.maybeStart(status(data))
    await settle()

    const final = '/ud/updates/0.0.9/Factr-I-0.0.9-mac-arm64.dmg'

    expect(dl.getState()).toEqual({ status: 'ready', version: '0.0.9', file: final, kind: 'dmg' })
    expect(mem.files.get(final)).toEqual(data)
    expect(mem.files.has(`${final}.partial`)).toBe(false)
    expect(mem.files.has('/ud/updates/0.0.7/old.dmg')).toBe(false)
    expect(states).toEqual(['downloading', 'ready'])
  })

  it('deletes the file and ends failed-silent on a hash mismatch', async () => {
    const data = bytes('installer-bytes-0123456789')
    const { dl, deps, mem } = setup()

    vi.mocked(deps.fetch).mockResolvedValue(ok(chunks(bytes('installer-bytes-0123456780'))))
    dl.maybeStart(status(data))
    await settle()

    expect(dl.getState().status).toBe('failed-silent')
    expect([...mem.files.keys()]).toEqual([])
  })

  it('treats a truncated download as failed and deletes the partial', async () => {
    const data = bytes('installer-bytes-0123456789')
    const { dl, deps, mem } = setup()

    vi.mocked(deps.fetch).mockResolvedValue(ok(chunks(data.slice(0, 10))))
    dl.maybeStart(status(data))
    await settle()

    expect(dl.getState().status).toBe('failed-silent')
    expect([...mem.files.keys()]).toEqual([])
  })

  it('rejects a body larger than the release says', async () => {
    const data = bytes('abc')
    const { dl, deps } = setup()

    vi.mocked(deps.fetch).mockResolvedValue(ok(chunks(bytes('abcdef'))))
    dl.maybeStart(status(data))
    await settle()

    expect(dl.getState().status).toBe('failed-silent')
  })

  it('follows redirects inside the allowlist', async () => {
    const data = bytes('installer-bytes-0123456789')
    const { dl, deps } = setup()

    vi.mocked(deps.fetch)
      .mockResolvedValueOnce(redirect('https://release-assets.githubusercontent.com/abc'))
      .mockResolvedValueOnce(ok(chunks(data)))
    dl.maybeStart(status(data))
    await settle()

    expect(dl.getState().status).toBe('ready')
    expect(vi.mocked(deps.fetch).mock.calls.map(c => c[0])).toEqual([URL_OK, 'https://release-assets.githubusercontent.com/abc'])
  })

  it('refuses a redirect off the allowlist', async () => {
    const data = bytes('installer-bytes-0123456789')
    const { dl, deps } = setup()

    vi.mocked(deps.fetch).mockResolvedValueOnce(redirect('https://evil.example/payload'))
    dl.maybeStart(status(data))
    await settle()

    expect(dl.getState().status).toBe('failed-silent')
    expect(deps.fetch).toHaveBeenCalledTimes(1)
  })

  it('refuses a non-https asset url', async () => {
    const data = bytes('x')
    const { dl, deps } = setup()

    dl.maybeStart(status(data, { url: 'http://github.com/Refactr-io/Factr-I/x.dmg' }))
    await settle()

    expect(deps.fetch).not.toHaveBeenCalled()
    expect(dl.getState().status).toBe('failed-silent')
  })

  it('does nothing while a turn is running, offline, or with the setting off', async () => {
    const data = bytes('abc')

    for (const gate of [{ isBusy: () => true }, { isOnline: () => false }, { isEnabled: () => false }]) {
      const { dl, deps } = setup(gate)

      dl.maybeStart(status(data))
      await settle()

      expect(deps.fetch).not.toHaveBeenCalled()
      expect(dl.getState().status).toBe('idle')
    }
  })

  it('ignoreEnabled (an explicit click with auto-update off) skips only the setting, not online or idle', async () => {
    const data = bytes('installer-bytes-0123456789')

    const off = setup({ isEnabled: () => false })

    vi.mocked(off.deps.fetch).mockResolvedValue(ok(chunks(data)))
    off.dl.maybeStart(status(data))
    await settle()
    expect(off.deps.fetch).not.toHaveBeenCalled()

    off.dl.maybeStart(undefined, { ignoreEnabled: true })
    await settle()
    expect(off.dl.getState().status).toBe('ready')

    for (const gate of [{ isBusy: () => true }, { isOnline: () => false }]) {
      const blocked = setup({ isEnabled: () => false, ...gate })

      blocked.dl.maybeStart(status(data), { ignoreEnabled: true })
      await settle()
      expect(blocked.deps.fetch).not.toHaveBeenCalled()
    }
  })

  it('does nothing without an asset or when no update is available', async () => {
    const { dl, deps } = setup()

    dl.maybeStart({ supported: true, updateAvailable: true, currentVersion: '0.0.8', latestVersion: '0.0.9', fetchedAt: 0 })
    dl.maybeStart({ ...status(bytes('a')), updateAvailable: false })
    await settle()

    expect(deps.fetch).not.toHaveBeenCalled()
  })

  it('pauses (aborts) when a turn starts, removes the partial, and resumes later', async () => {
    const data = bytes('installer-bytes-0123456789')
    let busy = false
    let release: () => void = () => undefined
    const gate = new Promise<void>(resolve => (release = resolve))
    const { dl, deps, mem } = setup({ isBusy: () => busy })

    vi.mocked(deps.fetch).mockResolvedValueOnce(ok(chunks(data, 3, async i => (i === 1 ? gate : undefined))))
    dl.maybeStart(status(data))
    await settle()
    expect(dl.getState().status).toBe('downloading')

    busy = true
    dl.pause()
    release()
    await settle()

    expect(dl.getState().status).toBe('idle')
    expect([...mem.files.keys()]).toEqual([])

    busy = false
    vi.mocked(deps.fetch).mockResolvedValueOnce(ok(chunks(data)))
    dl.maybeStart()
    await settle()

    expect(dl.getState().status).toBe('ready')
  })

  it('tries a version only once after a failure', async () => {
    const data = bytes('installer-bytes-0123456789')
    const { dl, deps } = setup()

    vi.mocked(deps.fetch).mockResolvedValue(ok(chunks(bytes('wrong-bytes-wrong-bytes!!'))))
    dl.maybeStart(status(data))
    await settle()
    dl.maybeStart(status(data))
    await settle()

    expect(deps.fetch).toHaveBeenCalledTimes(1)
  })

  it('reuses a complete verified file from an earlier run without downloading again', async () => {
    const data = bytes('installer-bytes-0123456789')
    const { dl, deps, mem } = setup()

    mem.files.set('/ud/updates/0.0.9/Factr-I-0.0.9-mac-arm64.dmg', data)
    dl.maybeStart(status(data))
    await settle()

    expect(dl.getState().status).toBe('ready')
    expect(deps.fetch).not.toHaveBeenCalled()
  })

  it('never throws when fetch itself throws', async () => {
    const data = bytes('abc')
    const { dl, deps } = setup()

    vi.mocked(deps.fetch).mockRejectedValue(new Error('offline'))

    expect(() => dl.maybeStart(status(data))).not.toThrow()
    await settle()
    expect(dl.getState().status).toBe('failed-silent')
  })
})

describe('adopt (a verified file from an earlier session)', () => {
  it('shows Ready at once when idle and never replaces a running download', async () => {
    const { createUpdateDownloader } = await import('./update-download')
    const seen: string[] = []
    const dl = createUpdateDownloader({
      fetch: async () => {
        throw new Error('no network at launch')
      },
      fs: {} as never,
      updatesDir: '/u',
      join: (...p: string[]) => p.join('/'),
      isEnabled: () => true,
      isOnline: () => false,
      isBusy: () => false,
      onState: s => void seen.push(s.status)
    })

    dl.adopt({ file: '/u/0.0.10/Factr-I-0.0.10-mac-arm64.zip', version: '0.0.10' })
    expect(dl.getState()).toMatchObject({ status: 'ready', version: '0.0.10', kind: 'zip' })
    expect(seen).toEqual(['ready'])
  })
})
