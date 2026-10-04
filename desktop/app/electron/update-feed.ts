/**
 * Update feed for the packaged app: asks GitHub for the latest published
 * release of Refactr-io/Factr-I and compares it with the running version.
 *
 * Pure of Electron (fetch, clock and version are injected) so it is unit
 * testable. There is no auto-install: "apply" only opens the release page.
 */

export const REPO_RELEASES_API = 'https://api.github.com/repos/Refactr-io/Factr-I/releases/latest'
export const RELEASE_URL_PREFIX = 'https://github.com/Refactr-io/Factr-I/'
export const RELEASES_PAGE_URL = `${RELEASE_URL_PREFIX}releases/latest`

const MIN_INTERVAL_MS = 6 * 60 * 60 * 1000
const FORCE_FLOOR_MS = 30 * 1000
const TIMEOUT_MS = 8000
const NOTES_MAX = 2000

export interface FeedStatus {
  supported: true
  updateAvailable: boolean
  currentVersion: string
  latestVersion?: string
  releaseUrl?: string
  notes?: string
  /** Set only when an update is available so the renderer's toast fires. */
  targetSha?: string
  error?: string
  message?: string
  fetchedAt: number
}

export interface FeedDeps {
  fetch: (url: string, init?: Record<string, unknown>) => Promise<FeedResponse>
  getVersion: () => string
  now?: () => number
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

export function isReleaseUrl(url: unknown): url is string {
  return typeof url === 'string' && url.startsWith(RELEASE_URL_PREFIX)
}

export function createUpdateFeed(deps: FeedDeps) {
  const now = deps.now ?? Date.now
  let etag: string | null = null
  let last: FeedStatus | null = null
  let lastAt = 0

  const base = (extra: Partial<FeedStatus>): FeedStatus => ({
    supported: true,
    updateAvailable: false,
    currentVersion: deps.getVersion(),
    fetchedAt: now(),
    ...extra
  })

  const fail = (message: string): FeedStatus =>
    last ? { ...last, error: 'check-failed', message } : base({ error: 'check-failed', message })

  async function check({ force = false }: { force?: boolean } = {}): Promise<FeedStatus> {
    const age = now() - lastAt

    if (last && !last.error && age < (force ? FORCE_FLOOR_MS : MIN_INTERVAL_MS)) {
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

      res = await deps.fetch(REPO_RELEASES_API, { headers, signal: controller.signal })
    } catch (error) {
      return fail(error instanceof Error ? error.message : String(error))
    } finally {
      clearTimeout(timer)
    }

    if (res.status === 304 && last) {
      lastAt = now()
      last = { ...last, fetchedAt: lastAt }

      return last
    }

    if (res.status === 404) {
      // No release published yet: nothing to offer, not an error.
      etag = null
      lastAt = now()
      last = base({})

      return last
    }

    if (res.status !== 200) {
      return fail(res.status === 403 || res.status === 429 ? 'GitHub rate limit reached.' : `GitHub returned ${res.status}.`)
    }

    let body: Record<string, unknown>

    try {
      body = (await res.json()) as Record<string, unknown>
    } catch {
      return fail('GitHub returned an unreadable response.')
    }

    etag = res.headers.get('etag')
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
    const notes = typeof body.body === 'string' && body.body.trim() ? body.body.trim().slice(0, NOTES_MAX) : undefined

    last = base({
      updateAvailable: available,
      latestVersion,
      releaseUrl: htmlUrl,
      ...(available ? { notes, targetSha: `v${latestVersion}` } : {})
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
