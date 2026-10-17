import { afterEach, describe, expect, it, vi } from 'vitest'

import {
  closeUpdateSplash,
  showUpdateSplash,
  type SplashWindowLike,
  UPDATE_SPLASH_BACKGROUND,
  updateSplashHtml
} from './update-splash'

const html = updateSplashHtml()
const style = /<style>([\s\S]*?)<\/style>/.exec(html)?.[1] ?? ''

describe('updateSplashHtml', () => {
  it('says only "Updating Factr-I" and "One moment...", with no version or digits', () => {
    const visible = html
      .replace(/<style>[\s\S]*?<\/style>/, '')
      .replace(/<svg[\s\S]*?<\/svg>/, '')
      .replace(/<title><\/title>/, '')
      .replace(/<[^>]+>/g, '|')
      .split('|')
      .filter(Boolean)

    expect(visible).toEqual(['Updating Factr-I', 'One moment...'])
    expect(html).not.toMatch(/v?\d+\.\d+\.\d+/)
  })

  it('has the reduced-motion rule that stops every animation, leaving the still full mascot', () => {
    const rule = /@media \(prefers-reduced-motion:reduce\)\{([\s\S]*?)\}(?=\s*(?:svg|\.|@|$))/.exec(style)?.[1] ?? ''

    expect(rule).toContain('animation:none!important')
  })

  it('animates only transform', () => {
    const frames = [...style.matchAll(/@keyframes [\w-]+\{([\s\S]*?)\}(?=\s*@keyframes|\s*@media|$)/g)].map(m => m[1] ?? '')
    const props = new Set<string>()

    for (const body of frames) {
      for (const decl of body.matchAll(/([a-z-]+):[^;{}]+(?=[;}])/g)) {
        props.add(decl[1] as string)
      }
    }

    expect(frames.length).toBe(2)

    for (const prop of props) {
      expect(['animation-timing-function', 'transform']).toContain(prop)
    }
  })

  it('is self-contained: no script, no external request, no filters or gradients', () => {
    expect(html).not.toMatch(/<script|https?:|@import|<image|<img|filter|gradient|box-shadow|drop-shadow/i)
    expect([...html.matchAll(/url\(([^)]*)\)/g)].every(m => m[1]?.startsWith('#'))).toBe(true)
  })

  it('is Blackthorn Ink on Mist Green and no other hue', () => {
    expect(style).toContain(UPDATE_SPLASH_BACKGROUND)
    expect(UPDATE_SPLASH_BACKGROUND).toBe('#D5E6DC')
    expect(html).toContain('fill="#151313"')
    expect(html).not.toMatch(/#67EAA2|#FFFDFC|#1E1E1D|#1D3023|#000000|#ffffff|#fff\b|#000\b/i)
  })

  it('loops in 3600ms', () => {
    expect(style).toContain('3600ms infinite')
  })
})

describe('splash window helpers', () => {
  const fake = () => {
    const win = { closed: false, close: vi.fn(() => void (win.closed = true)), isDestroyed: () => win.closed, loadURL: vi.fn(async (_url: string) => undefined) }

    return win satisfies SplashWindowLike & Record<string, unknown>
  }

  afterEach(() => closeUpdateSplash())

  it('opens one splash, loads the animation as a data url, and closes it', () => {
    const win = fake()
    const make = vi.fn(() => win)

    expect(showUpdateSplash(make)).toBe(win)
    expect(showUpdateSplash(make)).toBe(win)
    expect(make).toHaveBeenCalledTimes(1)
    expect(win.loadURL.mock.calls[0]?.[0]).toMatch(/^data:text\/html;charset=utf-8,/)

    closeUpdateSplash()
    expect(win.close).toHaveBeenCalledTimes(1)
    closeUpdateSplash()
    expect(win.close).toHaveBeenCalledTimes(1)
  })

  it('opens a frameless 360x260 window on the matching Mist Green background', () => {
    const win = fake()
    const make = vi.fn(() => win)

    showUpdateSplash(make)

    expect(make).toHaveBeenCalledWith(
      expect.objectContaining({ backgroundColor: '#D5E6DC', frame: false, height: 260, width: 360, resizable: false })
    )
  })
})
