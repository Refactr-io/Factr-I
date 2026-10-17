import { forwardRef, type ReactNode, useCallback, useEffect, useImperativeHandle, useRef } from 'react'

import { cn } from '@/lib/utils'

import { PAGE_INSET_X } from '../layout-constants'

import { SettingsStackContext } from './primitives'

export interface SettingsStackSection {
  id: string
  label: string
}

export interface SettingsStackHandle {
  scrollToSection: (id: string) => void
}

/** The page title above the stack: stays put while the sections scroll under it. */
export function SettingsPageTitle({ title }: { title: string }) {
  return (
    <header className={cn('shrink-0 pb-3', PAGE_INSET_X)}>
      <div className="mx-auto w-full max-w-[46rem]">
        <h1 className="text-xl font-semibold leading-7 tracking-[-0.01em] text-foreground">{title}</h1>
      </div>
    </header>
  )
}

// How far below the top of the scroller a section heading counts as "current".
const SPY_OFFSET_PX = 96

const prefersReducedMotion = () =>
  typeof window !== 'undefined' && window.matchMedia?.('(prefers-reduced-motion: reduce)').matches

/**
 * One continuous scrolling page per settings section: every sub-section is a block with a large heading, and the
 * sidebar mirrors those headings. `scrollToSection` (sidebar click) glides to a heading, instantly under reduced
 * motion; scrolling reports the current heading through `onActiveChange` (scroll-spy), and a click holds the spy
 * still until the glide has landed so the marker does not flicker through the sections in between.
 */
export const SettingsStack = forwardRef<
  SettingsStackHandle,
  {
    initialId?: string
    onActiveChange: (id: string) => void
    renderSection: (id: string) => ReactNode
    sections: readonly SettingsStackSection[]
  }
>(function SettingsStack({ initialId, onActiveChange, renderSection, sections }, ref) {
  const scrollerRef = useRef<HTMLDivElement>(null)
  const nodes = useRef(new Map<string, HTMLElement>())
  const lockUntil = useRef(0)
  const lastReported = useRef<string | null>(null)

  const report = useCallback(
    (id: string) => {
      if (lastReported.current !== id) {
        lastReported.current = id
        onActiveChange(id)
      }
    },
    [onActiveChange]
  )

  const targetTop = (id: string) => {
    const node = nodes.current.get(id)

    return node ? Math.max(0, node.offsetTop - 8) : 0
  }

  const scrollToSection = useCallback(
    (id: string) => {
      const scroller = scrollerRef.current

      if (!scroller || !nodes.current.has(id)) {
        return
      }

      const instant = prefersReducedMotion()
      lockUntil.current = Date.now() + (instant ? 80 : 700)
      report(id)
      scroller.scrollTo({ behavior: instant ? 'auto' : 'smooth', top: targetTop(id) })
    },
    [report]
  )

  useImperativeHandle(ref, () => ({ scrollToSection }), [scrollToSection])

  // Deep link (?page=): jump straight to that heading, then again once late content has settled the layout.
  // eslint-disable-next-line no-restricted-syntax -- scroll-spy lock timestamp, not an atom mirror
  useEffect(() => {
    if (!initialId || initialId === sections[0]?.id) {
      return
    }

    const jump = () => {
      const scroller = scrollerRef.current

      if (scroller && nodes.current.has(initialId)) {
        lockUntil.current = Date.now() + 120
        scroller.scrollTo({ behavior: 'auto', top: targetTop(initialId) })
        report(initialId)
      }
    }

    jump()
    const timer = window.setTimeout(jump, 350)

    return () => window.clearTimeout(timer)
    // Once per mount: the parent keys this component on the settings view.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const spy = useCallback(() => {
    const scroller = scrollerRef.current

    if (!scroller || Date.now() < lockUntil.current) {
      return
    }

    const atBottom = scroller.scrollTop + scroller.clientHeight >= scroller.scrollHeight - 4
    let current: string | undefined = sections[0]?.id

    if (atBottom && scroller.scrollTop > 0) {
      current = sections.at(-1)?.id
    } else {
      for (const section of sections) {
        const node = nodes.current.get(section.id)

        if (node && node.offsetTop - scroller.scrollTop <= SPY_OFFSET_PX) {
          current = section.id
        }
      }
    }

    if (current) {
      report(current)
    }
  }, [report, sections])

  return (
    <div
      className="relative h-full min-h-0 overflow-y-auto pb-[40vh]"
      data-settings-stack=""
      onScroll={spy}
      ref={scrollerRef}
    >
      <div className={cn('mx-auto w-full max-w-[46rem]', PAGE_INSET_X)}>
        {sections.map((section, index) => (
          <section
            className={cn('scroll-mt-4', index === 0 ? 'pt-4' : 'pt-12')}
            data-settings-section={section.id}
            id={`settings-section-${section.id}`}
            key={section.id}
            ref={node => {
              if (node) {
                nodes.current.set(section.id, node)
              } else {
                nodes.current.delete(section.id)
              }
            }}
          >
            {index > 0 && <div aria-hidden className="mb-10 h-px w-full rounded-full bg-(--ui-stroke-tertiary)" />}
            <h2 className="mb-5 text-[1.5rem] leading-8 font-semibold tracking-[-0.015em] text-foreground">
              {section.label}
            </h2>
            <SettingsStackContext.Provider value={{ index }}>{renderSection(section.id)}</SettingsStackContext.Provider>
          </section>
        ))}
      </div>
    </div>
  )
})
