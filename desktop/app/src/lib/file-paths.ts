import type { PreviewTarget } from '@/store/preview'

// A path-shaped token in prose: optional ~/ ./ ../ / prefix, segments, optional :line[:col].
// Lookbehind keeps URLs (`://`), emails and mid-word matches out.
export const PATH_RE = /(?<![\w@.~/:\\-])((?:~\/|\.{1,2}\/|\/)?[\w.@+-]+(?:\/[\w.@+-]+)*)(?::(\d+)(?::\d+)?)?(?![\w/])/g

export interface PathRef {
  line?: number
  path: string
}

/** A token worth an existence check: has a slash or a real file extension; not a version number or domain-ish word. */
export function parsePathRef(path: string, line?: string): PathRef | null {
  const clean = path.replace(/\.+$/, '')

  if (!clean || clean.includes('//') || /^\d+(\.\d+)+$/.test(clean) || /^www\./i.test(clean)) {
    return null
  }

  if (!clean.includes('/') && !/\.[A-Za-z][\w]{1,7}$/.test(clean)) {
    return null
  }

  return { line: line ? Number(line) : undefined, path: clean }
}

export function findPathRefs(text: string): { end: number; ref: PathRef; start: number; text: string }[] {
  const out: { end: number; ref: PathRef; start: number; text: string }[] = []

  for (const m of text.matchAll(PATH_RE)) {
    const ref = parsePathRef(m[1], m[2])

    if (ref) {
      // Trailing dots are sentence punctuation, not part of the path.
      const len = m[0].length - (m[1].length - ref.path.length)

      out.push({ end: m.index + len, ref, start: m.index, text: text.slice(m.index, m.index + len) })
    }
  }

  return out
}

const cache = new Map<string, Promise<PreviewTarget | null>>()

/** Resolve against cwd in the main process; null unless it is a real file. */
export function resolveExistingFile(path: string, cwd?: string | null): Promise<PreviewTarget | null> {
  const key = `${cwd ?? ''}\0${path}`
  let hit = cache.get(key)

  if (!hit) {
    hit = Promise.resolve(window.factrDesktop?.normalizePreviewTarget?.(path, cwd || undefined))
      .then(t => (t && t.kind === 'file' && (path.endsWith('index.html') || t.label !== 'index.html') ? t : null))
      .catch(() => null)

    if (cache.size > 500) {
      cache.clear()
    }

    cache.set(key, hit)
  }

  return hit
}
