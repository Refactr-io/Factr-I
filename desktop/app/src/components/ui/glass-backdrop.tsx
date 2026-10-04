import LiquidGlass from 'liquid-glass-react'
import { useEffect, useLayoutEffect, useRef, useState } from 'react'

import { cn } from '@/lib/utils'

const REDUCED = ['(prefers-reduced-motion: reduce)', '(prefers-reduced-transparency: reduce)'] as const

const isChromium = () => typeof navigator !== 'undefined' && /\bChrome\/\d/.test(navigator.userAgent)

/** Glass is only allowed when the platform can draw it and the user has not
 *  asked for less motion / less transparency. Re-evaluates on preference change. */
function useGlassAllowed(): boolean {
  const [allowed, setAllowed] = useState(false)

  useEffect(() => {
    if (typeof window.matchMedia !== 'function') {
      return
    }

    const lists = REDUCED.map(q => window.matchMedia(q))
    const sync = () => setAllowed(isChromium() && !lists.some(l => l.matches))

    sync()
    lists.forEach(l => l.addEventListener?.('change', sync))

    return () => lists.forEach(l => l.removeEventListener?.('change', sync))
  }, [])

  return allowed
}

function useIsLight(): boolean {
  const [light, setLight] = useState(() => !document.documentElement.classList.contains('dark'))

  useEffect(() => {
    const root = document.documentElement
    const sync = () => setLight(!root.classList.contains('dark'))
    const observer = new MutationObserver(sync)

    sync()
    observer.observe(root, { attributeFilter: ['class'], attributes: true })

    return () => observer.disconnect()
  }, [])

  return light
}

interface GlassBackdropProps {
  /** Blur/frost of the refracted backdrop (library `blurAmount`). */
  blurAmount?: number
  className?: string
  displacementScale?: number
  aberrationIntensity?: number
  /** Override the corner radius in px; defaults to the parent's computed radius. */
  radius?: number
}

/**
 * Decorative liquid-glass layer for an element that floats OVER live content.
 * Render it as the first child of a `relative` (or positioned) element; it
 * fills the element, clips to its rounded shape, and sits under its content.
 * Marks the parent with `data-glass` while active so CSS can thin the parent's
 * own fill. Renders nothing under reduced motion / transparency or outside
 * Chromium, so the parent's opaque surface stays and content never depends on it.
 */
export function GlassBackdrop({
  aberrationIntensity = 1.2,
  blurAmount = 0.06,
  className,
  displacementScale = 34,
  radius
}: GlassBackdropProps) {
  const allowed = useGlassAllowed()
  const light = useIsLight()
  const hostRef = useRef<HTMLDivElement>(null)
  const [box, setBox] = useState({ height: 0, radius: 0, width: 0 })

  useLayoutEffect(() => {
    const host = hostRef.current

    if (!allowed || !host) {
      return
    }

    const parent = host.parentElement
    parent?.setAttribute('data-glass', '')

    const measure = () => {
      const r = host.getBoundingClientRect()
      const cr = parent ? parseFloat(getComputedStyle(parent).borderTopLeftRadius) : 0

      setBox({
        height: Math.round(r.height),
        radius: radius ?? (Number.isFinite(cr) ? cr : 0),
        width: Math.round(r.width)
      })
    }

    measure()

    const observer = new ResizeObserver(measure)
    observer.observe(host)

    return () => {
      observer.disconnect()
      parent?.removeAttribute('data-glass')
    }
  }, [allowed, radius])

  if (!allowed) {
    return null
  }

  return (
    <div
      aria-hidden
      className={cn('pointer-events-none absolute inset-0 -z-10 overflow-hidden [border-radius:inherit]', className)}
      data-slot="glass-backdrop"
      ref={hostRef}
    >
      {box.width > 0 && box.height > 0 && (
        // The library only re-measures on window resize, so remount on size change.
        <LiquidGlass
          aberrationIntensity={aberrationIntensity}
          blurAmount={blurAmount}
          cornerRadius={box.radius}
          displacementScale={displacementScale}
          elasticity={0}
          key={`${box.width}x${box.height}`}
          overLight={light}
          padding="0"
          style={{ left: '50%', position: 'absolute', top: '50%' }}
        >
          <div style={{ height: box.height, width: box.width }} />
        </LiquidGlass>
      )}
    </div>
  )
}
