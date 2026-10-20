// @vitest-environment node
import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import { describe, expect, it } from 'vitest'

const app = resolve(dirname(fileURLToPath(import.meta.url)), '../..')
const css = readFileSync(join(app, 'src/styles.css'), 'utf8')
const fontSources = [...css.matchAll(/url\(['"]?(\.\/fonts\/[^'")]+)['"]?\)/g)].map(match => match[1])

function sourceFiles(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap(entry => {
    const path = join(dir, entry.name)

    return entry.isDirectory() ? sourceFiles(path) : /\.(?:tsx?|css)$/.test(path) && !path.includes('.test.') ? [path] : []
  })
}

describe('bundled typography', () => {
  it('resolves every declared font source to a real WOFF2 asset', () => {
    expect(fontSources).toHaveLength(13)

    for (const source of fontSources) {
      const path = resolve(app, 'src', source)
      expect(existsSync(path), source).toBe(true)
      expect(readFileSync(path).subarray(0, 4).toString(), source).toBe('wOF2')
    }
  })

  it('preloads the primary faces from the same URLs as their CSS declarations', () => {
    const html = readFileSync(join(app, 'index.html'), 'utf8')
    const preloads = [...html.matchAll(/<link rel="preload" href="([^"\s]+)" as="font"[^>]+crossorigin[^>]*>/g)]
    expect(preloads).toHaveLength(2)

    for (const [, href] of preloads) {
      expect(fontSources).toContain(`.${href.replace(/^\/src/, '')}`)
      expect(existsSync(join(app, href))).toBe(true)
    }

    expect(css).toMatch(/font-family: 'Pretendard';[^}]+font-display: block;/s)
  })

  it('has no retired UI family or hardcoded retired mono in components', () => {
    for (const path of sourceFiles(join(app, 'src'))) {
      const source = readFileSync(path, 'utf8')
      expect(source, path).not.toMatch(/\bArimo\b/)

      if (path.includes('/components/')) {
        expect(source, path).not.toMatch(/fontFamily:[^\n]*JetBrains|ctx\.font[^\n]*JetBrains/)
      }
    }
  })

  it('ships licenses and keeps the replacement font payload below 500 KiB', () => {
    for (const family of ['pretendard', 'ibm-plex-mono']) {
      expect(readFileSync(join(app, 'src/fonts', family, 'OFL.txt'), 'utf8')).toContain('SIL OPEN FONT LICENSE')
    }

    const total = fontSources.reduce((sum, source) => sum + statSync(resolve(app, 'src', source)).size, 0)
    expect(total).toBeLessThan(500 * 1024)
  })
})
