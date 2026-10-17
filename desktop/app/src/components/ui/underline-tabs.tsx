import * as React from 'react'

import { cn } from '@/lib/utils'

export interface UnderlineTab {
  id: string
  label: string
}

interface UnderlineTabsProps {
  ariaLabel: string
  className?: string
  onSelect: (id: string) => void
  tabs: readonly UnderlineTab[]
  value: string
}

/**
 * Page tabs: text on a hairline, with one ink indicator that slides to the active tab. The indicator only
 * animates transform and width (short ease-out, instant under reduced motion). Arrow keys, Home and End move
 * between tabs; focus shows as a fill on the tab, not a ring.
 */
export function UnderlineTabs({ ariaLabel, className, onSelect, tabs, value }: UnderlineTabsProps) {
  const listRef = React.useRef<HTMLDivElement>(null)
  const [bar, setBar] = React.useState<{ left: number; width: number } | null>(null)

  const measure = React.useCallback(() => {
    const active = listRef.current?.querySelector<HTMLElement>('[role="tab"][aria-selected="true"]')

    if (active) {
      setBar({ left: active.offsetLeft, width: active.offsetWidth })
    }
  }, [])

  React.useLayoutEffect(measure, [measure, value, tabs])

  React.useEffect(() => {
    const list = listRef.current

    if (!list || typeof ResizeObserver === 'undefined') {
      return
    }

    const observer = new ResizeObserver(measure)
    observer.observe(list)

    return () => observer.disconnect()
  }, [measure])

  const move = (event: React.KeyboardEvent, index: number) => {
    const last = tabs.length - 1

    const next =
      event.key === 'ArrowRight'
        ? (index + 1) % tabs.length
        : event.key === 'ArrowLeft'
          ? (index - 1 + tabs.length) % tabs.length
          : event.key === 'Home'
            ? 0
            : event.key === 'End'
              ? last
              : null

    if (next === null) {
      return
    }

    event.preventDefault()
    const target = tabs[next]

    if (target) {
      onSelect(target.id)
      listRef.current?.querySelectorAll<HTMLElement>('[role="tab"]')[next]?.focus()
    }
  }

  return (
    <div
      aria-label={ariaLabel}
      className={cn('no-scrollbar relative flex max-w-full gap-5 overflow-x-auto', className)}
      data-slot="underline-tabs"
      ref={listRef}
      role="tablist"
    >
      {tabs.map((tab, index) => {
        const active = tab.id === value

        return (
          <button
            aria-selected={active}
            className={cn(
              'h-9 shrink-0 whitespace-nowrap rounded-md px-0.5 text-sm transition-colors duration-150 focus-visible:bg-(--ui-control-active-background) motion-reduce:transition-none',
              active ? 'font-medium text-foreground' : 'text-(--ui-text-secondary) hover:text-foreground'
            )}
            key={tab.id}
            onClick={() => !active && onSelect(tab.id)}
            onKeyDown={event => move(event, index)}
            role="tab"
            tabIndex={active ? 0 : -1}
            type="button"
          >
            {tab.label}
          </button>
        )
      })}
      <span aria-hidden className="pointer-events-none absolute right-0 bottom-0 left-0 h-px bg-(--ui-stroke-tertiary)" />
      {bar ? (
        <span
          aria-hidden
          className="pointer-events-none absolute bottom-0 left-0 h-0.5 origin-left rounded-full bg-(--ui-control-on) transition-[transform,width] duration-200 ease-[cubic-bezier(0.23,1,0.32,1)] motion-reduce:transition-none"
          style={{ transform: `translateX(${bar.left}px)`, width: bar.width }}
        />
      ) : null}
    </div>
  )
}
