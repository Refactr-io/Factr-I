/**
 * Update feed for the packaged app: asks GitHub for the latest published
 * release of Refactr-io/Factr-I and compares it with the running version.
 *
 * Pure of Electron (fetch, clock and version are injected) so it is unit
 * testable. There is no silent self-replace: "apply" only opens the release page; the installer asset
 * (name, size, sha256 digest) is surfaced so update-download.ts can fetch and verify it in the background.
 */

export const REPO_RELEASES_API = 'https://api.github.com/repos/Refactr-io/Factr-I/releases/latest'
export const RELEASE_URL_PREFIX = 'https://github.com/Refactr-io/Factr-I/'
export const RELEASES_PAGE_URL = `${RELEASE_URL_PREFIX}releases/latest`

const MIN_INTERVAL_MS = 6 * 60 * 60 * 1000
const FORCE_FLOOR_MS = 30 * 1000
// After a failed check, wait this long before asking again (a forced check: FORCE_FLOOR_MS),
// or longer when GitHub says when to come back.
const FAIL_BACKOFF_MS = 15 * 60 * 1000
const MAX_SERVER_WAIT_MS = 24 * 60 * 60 * 1000
const TIMEOUT_MS = 8000
const NOTES_MAX = 2000

/** The installer for this platform, exactly as the GitHub release lists it. */
export interface UpdateAsset {
  name: string
  /** https://github.com/Refactr-io/Factr-I/releases/download/... (redirects to GitHub's asset CDN). */
  url: string
  size: number
  /** Lower-case hex SHA-256 from the release's `digest` ("sha256:<hex>"). */
  sha256: string
}

export interface FeedStatus {
  supported: true
  updateAvailable: boolean
  currentVersion: string
  latestVersion?: string
  releaseUrl?: string
  notes?: string
  /** Set only when an update is available so the renderer's toast fires. */
  targetSha?: string
  /** Set only when an update is available AND the release lists this platform's installer with a digest. */
  asset?: UpdateAsset
  error?: string
  message?: string
  fetchedAt: number
}

export interface FeedDeps {
  fetch: (url: string, init?: Record<string, unknown>) => Promise<FeedResponse>
  getVersion: () => string
  now?: () => number
  /** process.platform / process.arch (injectable for tests). */
  platform?: string
  arch?: string
  /** Override of the releases API URL. Only main wires this, and only when the app is not packaged. */
  apiUrl?: string
  openExternal: (url: string) => Promise<unknown>
}

interface FeedResponse {
  status: number
  headers: { get: (name: string) => string | null }
  json: () => Promise<unknown>
}

type Semver = { core: [number, number, number]; pre: string[] }

export function parseSemver(raw: unknown): Semver | null {
  const match = /^v?(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?$/.exec(String(raw ?? '').trim())

  if (!match) {
    return null
  }

  return { core: [Number(match[1]), Number(match[2]), Number(match[3])], pre: match[4] ? match[4].split('.') : [] }
}

/** Negative when a < b, 0 when equal, positive when a > b. */
export function compareSemver(a: Semver, b: Semver): number {
  for (let i = 0; i < 3; i++) {
    if (a.core[i] !== b.core[i]) {
      return a.core[i] - b.core[i]
    }
  }

  if (!a.pre.length || !b.pre.length) {
    return a.pre.length === b.pre.length ? 0 : a.pre.length ? -1 : 1
  }

  for (let i = 0; i < Math.max(a.pre.length, b.pre.length); i++) {
    const x = a.pre[i]
    const y = b.pre[i]

    if (x === undefined) {
      return -1
    }

    if (y === undefined) {
      return 1
    }

    const xn = /^\d+$/.test(x)
    const yn = /^\d+$/.test(y)

    if (xn && yn && Number(x) !== Number(y)) {
      return Number(x) - Number(y)
    }

    if (xn !== yn) {
      return xn ? -1 : 1
    }

    if (!xn && x !== y) {
      return x < y ? -1 : 1
    }
  }

  return 0
}

/**
 * How long GitHub asked us to wait, in ms (0 when it did not say): `Retry-After` (seconds or an
 * HTTP date), else `x-ratelimit-reset` (epoch seconds) when `x-ratelimit-remaining` is 0. Capped at 24 h.
 */
export function serverWaitMs(headers: { get: (name: string) => string | null }, nowMs: number): number {
  const clamp = (ms: number) => (Number.isFinite(ms) && ms > 0 ? Math.min(ms, MAX_SERVER_WAIT_MS) : 0)
  const retryAfter = headers.get('retry-after')?.trim()

  if (retryAfter) {
    if (/^\d+$/.test(retryAfter)) {
      return clamp(Number(retryAfter) * 1000)
    }

    const at = Date.parse(retryAfter)

    if (!Number.isNaN(at)) {
      return clamp(at - nowMs)
    }
  }

  const reset = headers.get('x-ratelimit-reset')?.trim()

  if (reset && /^\d+$/.test(reset) && headers.get('x-ratelimit-remaining')?.trim() === '0') {
    return clamp(Number(reset) * 1000 - nowMs)
  }

  return 0
}

export function isReleaseUrl(url: unknown): url is string {
  return typeof url === 'string' && url.startsWith(RELEASE_URL_PREFIX)
}

/** "sha256:<64 hex>" -> the lower-case hex, else null. */
export function parseDigest(raw: unknown): string | null {
  const match = /^sha256:([0-9a-fA-F]{64})$/.exec(String(raw ?? '').trim())

  return match ? match[1].toLowerCase() : null
}

/** The installer file name electron-builder gives this platform/arch (see package.json artifactName). */
export function expectedAssetName(version: string, platform: string, arch: string): string | null {
  if (platform === 'darwin' && arch === 'arm64') {
    return `Factr-I-${version}-mac-arm64.dmg`
  }

  if (platform === 'win32' && arch === 'x64') {
    return `Factr-I-${version}-win-x64.exe`
  }

  if (platform === 'linux' && arch === 'x64') {
    return `Factr-I-${version}-linux-x86_64.AppImage`
  }

  return null
}

/**
 * Pick this platform's installer from a release's assets by EXACT name, and require its size and digest.
 * A missing asset, size or digest means "notify only": the caller keeps the old behaviour.
 */
export function pickUpdateAsset(
  assets: unknown,
  version: string,
  platform: string,
  arch: string,
  urlOk: (url: unknown) => url is string = isReleaseUrl
): UpdateAsset | null {
  const wanted = expectedAssetName(version, platform, arch)

  if (!wanted || !Array.isArray(assets)) {
    return null
  }

  const found = assets.find(a => a && typeof a === 'object' && (a as { name?: unknown }).name === wanted) as
    | { browser_download_url?: unknown; digest?: unknown; size?: unknown }
    | undefined

  const sha256 = parseDigest(found?.digest)
  const size = Number(found?.size)
  const url = found?.browser_download_url

  if (!found || !sha256 || !Number.isInteger(size) || size <= 0 || !urlOk(url)) {
    return null
  }

  return { name: wanted, url, size, sha256 }
}

export function createUpdateFeed(deps: FeedDeps) {
  const now = deps.now ?? Date.now
  // The dev hook (apiUrl) may serve its fake asset from the same local origin; release builds never do.
  const devOrigin = deps.apiUrl ? new URL(deps.apiUrl).origin : null

  const assetUrlOk = (url: unknown): url is string =>
    isReleaseUrl(url) || (devOrigin !== null && typeof url === 'string' && url.startsWith(`${devOrigin}/`))

  let etag: string | null = null
  let last: FeedStatus | null = null
  let lastAt = 0
  // The most recent failed check, kept until a check succeeds: repeated calls (renderer polls,
  // window focus) return it instead of hitting GitHub again until the backoff has passed.
  let failure: FeedStatus | null = null
  let failedAt = 0
  let retryAt = 0

  const base = (extra: Partial<FeedStatus>): FeedStatus => ({
    supported: true,
    updateAvailable: false,
    currentVersion: deps.getVersion(),
    fetchedAt: now(),
    ...extra
  })

  const fail = (message: string, waitMs = 0): FeedStatus => {
    failedAt = now()
    retryAt = failedAt + waitMs
    failure = last ? { ...last, error: 'check-failed', message } : base({ error: 'check-failed', message })

    return failure
  }

  const succeeded = () => {
    failure = null
    retryAt = 0
  }

  async function check({ force = false }: { force?: boolean } = {}): Promise<FeedStatus> {
    const t = now()

    // A server-requested wait binds forced checks too; otherwise back off after any failure.
    if (failure && (t < retryAt || t - failedAt < (force ? FORCE_FLOOR_MS : FAIL_BACKOFF_MS))) {
      return failure
    }

    const age = t - lastAt

    if (last && !failure && age < (force ? FORCE_FLOOR_MS : MIN_INTERVAL_MS)) {
      return last
    }

    const controller = new AbortController()
    const timer = setTimeout(() => controller.abort(), TIMEOUT_MS)

    let res: FeedResponse

    try {
      const headers: Record<string, string> = {
        Accept: 'application/vnd.github+json',
        'User-Agent': 'Factr-I-desktop',
        'X-GitHub-Api-Version': '2022-11-28'
      }

      if (etag && last) {
        headers['If-None-Match'] = etag
      }

      res = await deps.fetch(deps.apiUrl || REPO_RELEASES_API, { headers, signal: controller.signal })
    } catch (error) {
      return fail(error instanceof Error ? error.message : String(error))
    } finally {
      clearTimeout(timer)
    }

    if (res.status === 304 && last) {
      succeeded()
      lastAt = now()
      last = { ...last, fetchedAt: lastAt }

      return last
    }

    if (res.status === 404) {
      // No release published yet: nothing to offer, not an error.
      etag = null
      succeeded()
      lastAt = now()
      last = base({})

      return last
    }

    if (res.status !== 200) {
      return fail(
        res.status === 403 || res.status === 429 ? 'GitHub rate limit reached.' : `GitHub returned ${res.status}.`,
        serverWaitMs(res.headers, now())
      )
    }

    let body: Record<string, unknown>

    try {
      body = (await res.json()) as Record<string, unknown>
    } catch {
      return fail('GitHub returned an unreadable response.')
    }

    etag = res.headers.get('etag')
    succeeded()
    lastAt = now()

    const latest = parseSemver(body?.tag_name)
    const current = parseSemver(deps.getVersion())

    if (!body || body.draft === true || body.prerelease === true || !latest || !current) {
      last = base({})

      return last
    }

    const latestVersion = String(body.tag_name).trim().replace(/^v/, '')
    const tag = String(body.tag_name).trim()
    const htmlUrl = isReleaseUrl(body.html_url) ? body.html_url : `${RELEASE_URL_PREFIX}releases/tag/${encodeURIComponent(tag)}`
    const available = compareSemver(latest, current) > 0

    const asset = available
      ? pickUpdateAsset(body.assets, latestVersion, deps.platform ?? process.platform, deps.arch ?? process.arch, assetUrlOk)
      : null

    const notes = typeof body.body === 'string' && body.body.trim() ? body.body.trim().slice(0, NOTES_MAX) : undefined

    last = base({
      updateAvailable: available,
      latestVersion,
      releaseUrl: htmlUrl,
      ...(available ? { notes, targetSha: `v${latestVersion}`, ...(asset ? { asset } : {}) } : {})
    })

    return last
  }

  async function apply() {
    const url = last?.updateAvailable && isReleaseUrl(last.releaseUrl) ? last.releaseUrl : RELEASES_PAGE_URL

    try {
      await deps.openExternal(url)
    } catch (error) {
      return { ok: false, error: 'open-failed', message: error instanceof Error ? error.message : String(error) }
    }

    return { ok: true, openedReleasePage: true, message: 'Opened the release page in your browser.' }
  }

  return { check, apply }
}
