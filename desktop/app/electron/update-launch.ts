/**
 * Launch-time half of "Restart to update" (macOS): find a downloaded, verified update without any network, and
 * answer the new app's "was I just updated" question. Plain synchronous fs, so main.ts can call it before the
 * first window exists.
 */

import fs from 'node:fs'
import path from 'node:path'

import { parseSemver } from './update-feed'
import type { PostUpdate, ReadyUpdate } from './update-swap'

const compare = (a: string, b: string): number => {
  const x = parseSemver(a)
  const y = parseSemver(b)

  if (!x || !y) {
    return 0
  }

  for (let i = 0; i < 3; i += 1) {
    if (x.core[i] !== y.core[i]) {
      return x.core[i] < y.core[i] ? -1 : 1
    }
  }

  return 0
}

/**
 * The newest verified mac zip under <updatesDir>/<version>/ that is newer than the running app. The downloader
 * renames a file to its final name only after size and sha256 match, so a final name means verified. A version
 * whose swap was already rolled back, or that was tried this launch, is skipped so a bad update cannot loop.
 */
export function findReadyZip(updatesDir: string, currentVersion: string): null | ReadyUpdate {
  let best: null | ReadyUpdate = null

  let versions: string[]

  try {
    versions = fs.readdirSync(updatesDir)
  } catch {
    return null
  }

  for (const version of versions) {
    if (!parseSemver(version) || compare(version, currentVersion) <= 0) {
      continue
    }

    const dir = path.join(updatesDir, version)
    const file = path.join(dir, `Factr-I-${version}-mac-arm64.zip`)

    if (!fs.existsSync(file) || fs.existsSync(path.join(dir, 'rolledback'))) {
      continue
    }

    if (!best || compare(version, best.version) > 0) {
      best = { file, kind: 'zip', version }
    }
  }

  return best
}

/** What the helper left for the app it just launched, only when this is the version it installed. */
export function readPostUpdateSync(updatesDir: string, currentVersion: string): null | PostUpdate {
  try {
    const parsed = JSON.parse(fs.readFileSync(path.join(updatesDir, 'post-update.json'), 'utf8')) as Partial<PostUpdate>

    if (typeof parsed.to === 'string' && typeof parsed.from === 'string' && typeof parsed.marker === 'string' && parsed.to === currentVersion) {
      return { from: parsed.from, marker: parsed.marker, to: parsed.to }
    }
  } catch {
    // no record
  }

  return null
}

/** Tell the helper this build came up healthy (it then deletes the backup of the old app). */
export function writeHealthMarker(record: PostUpdate): void {
  try {
    fs.mkdirSync(path.dirname(record.marker), { recursive: true })
    fs.writeFileSync(record.marker, '')
  } catch {
    // The helper rolls back if it never sees this; nothing else to do.
  }
}

export function clearPostUpdate(updatesDir: string): void {
  try {
    fs.rmSync(path.join(updatesDir, 'post-update.json'), { force: true })
  } catch {
    // ignore
  }
}
