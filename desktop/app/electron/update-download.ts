/**
 * Background download of the installer for an available update. Notify + download + verify + one-click install:
 * the app is ad-hoc signed, so nothing here replaces the running app. A verified file is only ever OPENED for the
 * user (the .dmg in Finder, the installer on Windows) and nothing installs without them.
 *
 * Pure of Electron (fetch, fs, clock gates injected) so it is unit testable. It never throws to its caller: any
 * failure ends in `failed-silent` and the UI falls back to the release page.
 *
 * Honest scope: the expected SHA-256 comes from the same GitHub release as the file. It protects against
 * corruption and truncation, not against a compromised repository.
 */

import { createHash } from 'node:crypto'

import type { FeedStatus, UpdateAsset } from './update-feed'

export type DownloadStatus = 'downloading' | 'failed-silent' | 'idle' | 'ready'

export interface DownloadState {
  status: DownloadStatus
  version: null | string
  /** Absolute path of the verified installer when status is 'ready'. */
  file: null | string
}

export interface DownloadFetchResponse {
  status: number
  headers: { get: (name: string) => null | string }
  body: AsyncIterable<Uint8Array> | null
}

export interface DownloaderFs {
  mkdir: (dir: string) => Promise<void>
  /** Removes a file or folder tree; a missing path is not an error. */
  remove: (target: string) => Promise<void>
  list: (dir: string) => Promise<string[]>
  size: (file: string) => Promise<null | number>
  rename: (from: string, to: string) => Promise<void>
  open: (file: string) => Promise<{ close: () => Promise<void>; write: (chunk: Uint8Array) => Promise<void> }>
  /** Streams an existing file for hashing. */
  read: (file: string) => AsyncIterable<Uint8Array>
}

export interface DownloaderDeps {
  fetch: (url: string, init: { redirect: 'manual'; signal: AbortSignal }) => Promise<DownloadFetchResponse>
  fs: DownloaderFs
  /** <userData>/updates */
  updatesDir: string
  /** Join helper (path.join) so the module stays platform-neutral. */
  join: (...parts: string[]) => string
  isEnabled: () => boolean
  isOnline: () => boolean
  /** True while an agent turn or tool call is running (the quit guard's active-work signal). */
  isBusy: () => boolean
  /** Dev hook only: an extra origin (the local fake release server) that may serve the asset over plain http. */
  devOrigin?: null | string
  log?: (message: string) => void
  onState?: (state: DownloadState) => void
}

const ALLOWED_HOSTS = new Set(['github.com', 'objects.githubusercontent.com', 'release-assets.githubusercontent.com'])
const MAX_REDIRECTS = 5

const IDLE: DownloadState = { status: 'idle', version: null, file: null }

/** https only, and only GitHub's own hosts (the dev origin aside). */
export function isAllowedDownloadUrl(raw: string, devOrigin?: null | string): boolean {
  let url: URL

  try {
    url = new URL(raw)
  } catch {
    return false
  }

  if (devOrigin && url.origin === devOrigin) {
    return true
  }

  return url.protocol === 'https:' && ALLOWED_HOSTS.has(url.hostname)
}

export function createUpdateDownloader(deps: DownloaderDeps) {
  let state: DownloadState = IDLE
  let controller: AbortController | null = null
  let cleaned = false
  // Versions whose download already failed this run: tried once, then left to the release page.
  const failed = new Set<string>()
  let latest: FeedStatus | null = null
  let running: null | Promise<void> = null

  const log = (message: string) => deps.log?.(`[update-download] ${message}`)

  const setState = (next: DownloadState) => {
    state = next
    deps.onState?.(state)
  }

  async function hashFile(file: string): Promise<string> {
    const hash = createHash('sha256')

    for await (const chunk of deps.fs.read(file)) {
      hash.update(chunk)
    }

    return hash.digest('hex')
  }

  async function removeOtherVersions(version: string) {
    try {
      for (const entry of await deps.fs.list(deps.updatesDir)) {
        if (entry !== version) {
          await deps.fs.remove(deps.join(deps.updatesDir, entry))
        }
      }
    } catch (error) {
      log(`cleanup skipped: ${String(error)}`)
    }
  }

  /** GET with manual redirects so every hop is checked against the allowlist. */
  async function fetchFollowing(url: string, signal: AbortSignal): Promise<DownloadFetchResponse> {
    let current = url

    for (let hop = 0; hop <= MAX_REDIRECTS; hop += 1) {
      if (!isAllowedDownloadUrl(current, deps.devOrigin)) {
        throw new Error(`blocked url: ${current}`)
      }

      const res = await deps.fetch(current, { redirect: 'manual', signal })

      if (res.status >= 300 && res.status < 400) {
        const location = res.headers.get('location')

        if (!location) {
          throw new Error('redirect without a location')
        }

        current = new URL(location, current).toString()

        continue
      }

      return res
    }

    throw new Error('too many redirects')
  }

  async function download(asset: UpdateAsset, version: string) {
    const dir = deps.join(deps.updatesDir, version)
    const finalPath = deps.join(dir, asset.name)
    const partialPath = `${finalPath}.partial`

    await deps.fs.mkdir(dir)

    // An earlier run may have left a complete, verified file: reuse it.
    if ((await deps.fs.size(finalPath)) === asset.size && (await hashFile(finalPath)) === asset.sha256) {
      setState({ status: 'ready', version, file: finalPath })

      return
    }

    await deps.fs.remove(partialPath)
    setState({ status: 'downloading', version, file: null })

    controller = new AbortController()
    const { signal } = controller
    const hash = createHash('sha256')
    let received = 0
    let handle: Awaited<ReturnType<DownloaderFs['open']>> | null = null

    try {
      const res = await fetchFollowing(asset.url, signal)

      if (res.status !== 200 || !res.body) {
        throw new Error(`download returned ${res.status}`)
      }

      handle = await deps.fs.open(partialPath)

      for await (const chunk of res.body) {
        if (signal.aborted) {
          throw new Error('aborted')
        }

        received += chunk.byteLength

        if (received > asset.size) {
          throw new Error('larger than the release says')
        }

        hash.update(chunk)
        await handle.write(chunk)
      }

      await handle.close()
      handle = null

      if (received !== asset.size) {
        throw new Error(`truncated: ${received} of ${asset.size} bytes`)
      }

      if (hash.digest('hex') !== asset.sha256) {
        throw new Error('sha256 mismatch')
      }

      await deps.fs.rename(partialPath, finalPath)
      setState({ status: 'ready', version, file: finalPath })
      log(`ready ${asset.name}`)
    } catch (error) {
      await handle?.close().catch(() => undefined)
      await deps.fs.remove(partialPath)

      if (signal.aborted) {
        // Paused (a turn started, the setting went off, the app is quitting): not a failure, try again later.
        setState(IDLE)
      } else {
        failed.add(version)
        setState({ status: 'failed-silent', version, file: null })
        log(`failed ${asset.name}: ${error instanceof Error ? error.message : String(error)}`)
      }
    } finally {
      controller = null
    }
  }

  /** Start (or resume) the download for the latest feed status when every gate is open. Never throws. */
  function maybeStart(status?: FeedStatus | null): void {
    if (status) {
      latest = status
    }

    const feed = latest
    const asset = feed?.updateAvailable ? feed.asset : undefined
    const version = feed?.latestVersion

    if (!feed || !asset || !version || running || !deps.isEnabled() || !deps.isOnline() || deps.isBusy()) {
      return
    }

    if (failed.has(version) || (state.version === version && (state.status === 'ready' || state.status === 'downloading'))) {
      return
    }

    running = (async () => {
      try {
        if (!cleaned) {
          cleaned = true
          await removeOtherVersions(version)
        }

        await download(asset, version)
      } catch (error) {
        log(`unexpected: ${String(error)}`)
        setState({ status: 'failed-silent', version, file: null })
      } finally {
        running = null
      }
    })()
  }

  /** Abort a running download (a turn started, the setting went off, the app is quitting). */
  function pause(): void {
    controller?.abort()
  }

  return { getState: () => state, maybeStart, pause }
}
