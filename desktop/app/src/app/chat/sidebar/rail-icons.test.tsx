import { render } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { Activity, Apps, Bot, CalendarEvent, Files, Home, MessageCircle, RAIL_ICON_PROPS } from '@/lib/icons'

describe('rail icon geometry', () => {
  it('pins every outline to the same size, stroke and centered Tabler viewBox', () => {
    expect(RAIL_ICON_PROPS).toEqual({ className: 'size-[1.125rem]', size: 18, stroke: 1.5 })

    for (const Icon of [Home, Apps, MessageCircle, Files, CalendarEvent, Activity, Bot]) {
      const { container, unmount } = render(<Icon {...RAIL_ICON_PROPS} />)
      const svg = container.querySelector('svg')!
      expect(svg.getAttribute('width')).toBe('18')
      expect(svg.getAttribute('height')).toBe('18')
      expect(svg.getAttribute('stroke-width')).toBe('1.5')
      expect(svg.getAttribute('viewBox')).toBe('0 0 24 24')
      expect(svg.getAttribute('stroke-linecap')).toBe('round')
      unmount()
    }
  })
  it('uses the bot-with-speech-bubble mark, not a profile avatar', () => {
    const { container } = render(<Bot {...RAIL_ICON_PROPS} />)
    const svg = container.querySelector('svg')
    expect(svg?.getAttribute('viewBox')).toBe('0 0 24 24')
    // The antenna, ears and bubble of the drawn mark (lib/drawn-icons.tsx).
    expect(svg?.querySelectorAll('path').length).toBe(6)
  })
})
