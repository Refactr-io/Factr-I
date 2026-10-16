import { cleanup, render } from '@testing-library/react'
import { afterEach, describe, expect, it } from 'vitest'

import { HelmetMark } from './helmet-mark'

const layers = (container: HTMLElement) => ({
  ink: container.querySelector('[data-layer="ink"]') as SVGPathElement,
  light: container.querySelector('[data-layer="light"]') as SVGPathElement
})

describe('HelmetMark', () => {
  afterEach(cleanup)

  it('always draws the art in Blackthorn Ink, with the star eyes as evenodd holes', () => {
    const { ink } = layers(render(<HelmetMark />).container)

    expect(ink.getAttribute('fill')).toBe('#151313')
    expect(ink.getAttribute('fill-rule')).toBe('evenodd')
  })

  it('puts a Rose Alloy silhouette with a 40-unit round stroke under the art, shown only in the dark theme by default', () => {
    const { light } = layers(render(<HelmetMark />).container)

    expect(light.getAttribute('fill')).toBe('#FFFDFC')
    expect(light.getAttribute('stroke')).toBe('#FFFDFC')
    expect(light.getAttribute('stroke-width')).toBe('40')
    expect(light.getAttribute('stroke-linejoin')).toBe('round')
    expect(light.getAttribute('class')).toBe('hidden dark:block')
  })

  it('tone dark always shows the light layer and tone light never does', () => {
    expect(layers(render(<HelmetMark tone="dark" />).container).light.getAttribute('class')).toBe('block')
    cleanup()
    expect(layers(render(<HelmetMark tone="light" />).container).light.getAttribute('class')).toBe('hidden')
  })

  it('uses no pure black or white', () => {
    const { container } = render(<HelmetMark />)

    expect(container.innerHTML).not.toMatch(/#000000|#ffffff|#fff\b|#000\b/i)
  })
})
