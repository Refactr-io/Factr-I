import { readdirSync, readFileSync, statSync } from 'node:fs'
import { join } from 'node:path'

import { describe, expect, it } from 'vitest'

import { $zoomPercent } from '@/store/zoom'

const here = __dirname
const read = (...parts: string[]) => readFileSync(join(here, ...parts), 'utf8')

describe('Appearance keeps Factr font customisation on top of the calibrated defaults', () => {
  const appearance = read('appearance-settings.tsx')

  it('exposes UI scale presets, the chat font picker and the terminal font setting', () => {
    expect(appearance).toContain("['90', '100', '110', '125', '150', '175']")
    expect(appearance).toContain('<ChatFontSetting />')
    expect(appearance).toContain('<TerminalFontSetting />')
    expect(appearance).toContain("appearanceSettingElementId('desktop.font_family')")
    expect(appearance).toContain("appearanceSettingElementId('terminal.font_family')")
  })

  it('starts at 100% zoom, so the calibrated sizes are what a fresh profile sees', () => {
    expect(read('..', '..', '..', 'electron', 'zoom.ts')).toMatch(/export const DEFAULT_ZOOM_LEVEL = 0\b/)
    expect($zoomPercent.get()).toBe(100)
  })

  it('ships Arimo with the Claude Code calibrated sizes and no scaling tricks', () => {
    const css = read('..', '..', 'styles.css')

    // Claude Code's whole-pixel scale: body 14, UI 13, tool and caption lines 12.
    expect(css).toMatch(/--conversation-text-font-size:\s*0\.875rem/)
    expect(css).toMatch(/--conversation-tool-font-size:\s*0\.75rem/)
    expect(css).toMatch(/--conversation-caption-font-size:\s*0\.75rem/)
    expect(css).not.toMatch(/size-adjust|font-size-adjust/)
    expect(css).toContain("--dt-font-sans:\n      'Arimo'")
  })

  it('keeps the font family key wired to live apply and autosave', () => {
    const font = read('chat-font-setting.tsx')
    const theme = read('..', '..', 'themes', 'context.tsx')

    expect(font).toContain('desktop.font_family')
    expect(theme).toContain("'--dt-font-sans': resolveChatFontFamily(")
  })
})

describe('type scale uses whole-pixel font sizes only', () => {
  const root = join(here, '..', '..')

  const files = (dir: string): string[] =>
    readdirSync(dir).flatMap(name => {
      const path = join(dir, name)

      if (statSync(path).isDirectory()) {
        return name === 'fonts' || name === 'node_modules' ? [] : files(path)
      }

      return /\.(css|tsx?)$/.test(name) && !/\.test\./.test(name) ? [path] : []
    })

  it('has no decimal px font size in any token, class or font-size declaration', () => {
    const offenders: string[] = []

    for (const file of files(root)) {
      const lines = readFileSync(file, 'utf8').split('\n')

      lines.forEach((line, index) => {
        for (const match of line.matchAll(
          /text-\[([0-9.]+)(rem|px)\]|(?:font-size|--text-(?:xs|sm|base|lg)|--conversation-[a-z-]*font-size):\s*([0-9.]+)(rem|px)\b/g
        )) {
          const value = Number(match[1] ?? match[3])
          const px = (match[2] ?? match[4]) === 'rem' ? value * 16 : value

          if (Math.abs(px - Math.round(px)) > 1e-6) {
            offenders.push(`${file.slice(root.length)}:${index + 1} ${match[0]}`)
          }
        }
      })
    }

    expect(offenders).toEqual([])
  })
})
