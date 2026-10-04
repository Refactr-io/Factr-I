/**
 * Lockstep check for the three parts of one packaged build: the Factr engine binary, the bundled
 * Factr Python source and the Python runtime. `scripts/factr-manifest.mjs` writes
 * `resources/factr/manifest.json` at pack time; the engine reports its own build on
 * `GET /api/status`. Any engine that does not match its bundle (a process left over from another
 * install, a half-copied update) is refused instead of silently run against the wrong Python source.
 *
 * Pure so it is testable without Electron.
 */

export interface FactrManifest {
  /** Unique per pack; the update script and the launch marker key on it. */
  id: string
  engine: { sha: null | string; version: string }
  factr: { dirty?: boolean; sha: null | string }
  python: { version: string }
}

/** What the engine's public `/api/status` says about itself. */
export interface EngineStatus {
  engine?: unknown
  sha?: unknown
  version?: unknown
}

export function parseFactrManifest(raw: null | string): null | FactrManifest {
  try {
    const value = JSON.parse(raw ?? '')

    return typeof value?.id === 'string' && typeof value?.engine?.version === 'string' ? (value as FactrManifest) : null
  } catch {
    return null
  }
}

// `git rev-parse --short` lengths differ between clones: equal when one is a prefix of the other.
const sameSha = (a: string, b: string) => a.length >= 7 && b.length >= 7 && (a.startsWith(b) || b.startsWith(a))

/** Null when the running engine is the bundled one, else a message for the boot-failure screen. */
export function engineMismatch(manifest: null | FactrManifest, status: EngineStatus): null | string {
  if (!manifest) {
    return null
  }

  const want = `${manifest.engine.version}${manifest.engine.sha ? ` (${manifest.engine.sha})` : ''}`

  if (status.engine !== 'factr') {
    return `Something other than the bundled Factr engine ${want} answered. Quit Factr-I, stop any leftover Factr-I or factr process, then reopen it.`
  }

  const version = typeof status.version === 'string' ? status.version : ''
  const sha = typeof status.sha === 'string' ? status.sha : ''

  if (version !== manifest.engine.version || (manifest.engine.sha && !sameSha(manifest.engine.sha, sha))) {
    return `The running Factr engine (${version || 'unknown'}${sha ? ` ${sha}` : ''}) does not match this app's bundle (${want}). It is probably left over from another install. Quit Factr-I, stop any leftover factr process, then reopen it; if it persists, reinstall Factr-I.`
  }

  return null
}
