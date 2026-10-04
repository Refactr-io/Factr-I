import { animate, motion, useMotionTemplate, useMotionValue, useReducedMotion } from 'motion/react'
import * as React from 'react'

import type { IconComponent } from '@/lib/icons'
import { cn } from '@/lib/utils'

export interface SegmentedControlOption<T extends string> {
  id: T
  label: string
  icon?: IconComponent
}

interface SegmentedControlProps<T extends string> {
  options: readonly SegmentedControlOption<T>[]
  value: T
  onChange: (id: T) => void
  className?: string
  /** Dims the whole track and blocks selection (e.g. gated behind a prerequisite). */
  disabled?: boolean
}

interface Slot {
  left: number
  right: number
}

const EASE_OUT = [0.23, 1, 0.32, 1] as const
const DILATE_MS = 190
const CONTRACT_DELAY_MS = 150
const OVERSHOOT_PX = 3
const DEADZONE_PX = 4
const RUBBER = 0.25
const FLICK_PX_PER_MS = 0.5
const THUMB_RADIUS_PX = 4

const slotClass = 'flex items-center justify-center gap-1 px-2.5 py-0.5 text-[0.6875rem] font-medium'

/**
 * Grouped one-row toggle used for small mutually-exclusive choices
 * (color mode, tool-call display, usage period, etc.). The thumb is a single
 * clipped layer over a copy of the labels: a tap first dilates it across the
 * old and new slot, then contracts onto the new one, and it can be dragged
 * (rubber-banding past the ends, flick to snap). Adapted from React Bits'
 * RubberSegment.
 */
export function SegmentedControl<T extends string>({
  className,
  disabled = false,
  onChange,
  options,
  value
}: SegmentedControlProps<T>) {
  const reduceMotion = useReducedMotion()
  const innerRef = React.useRef<HTMLDivElement>(null)
  const buttonRefs = React.useRef<(HTMLButtonElement | null)[]>([])
  const slots = React.useRef<Slot[]>([])
  // Distinguishes our own selections (animate) from the parent changing `value` (jump).
  const pendingId = React.useRef<T | null>(null)
  const runId = React.useRef(0)

  const drag = React.useRef<{
    startX: number
    startLeft: number
    lastX: number
    lastT: number
    velocity: number
    active: boolean
    pointerId: number
  } | null>(null)

  const [ready, setReady] = React.useState(false)

  const left = useMotionValue(0)
  const rightInset = useMotionValue(0)
  const width = useMotionValue(0)
  const clipPath = useMotionTemplate`inset(0px ${rightInset}px 0px ${left}px round ${THUMB_RADIUS_PX}px)`

  const activeIndex = Math.max(
    0,
    options.findIndex(o => o.id === value)
  )

  const jumpTo = React.useCallback(
    (index: number) => {
      const slot = slots.current[index]

      if (!slot) {
        return
      }

      runId.current += 1
      left.set(slot.left)
      rightInset.set(width.get() - slot.right)
    },
    [left, rightInset, width]
  )

  const glideTo = React.useCallback(
    async (index: number) => {
      const target = slots.current[index]

      if (!target) {
        return
      }

      const run = ++runId.current
      const alive = () => runId.current === run
      const targetInset = width.get() - target.right
      const fromLeft = left.get()
      const fromInset = rightInset.get()
      // Which edge leads depends on whether the thumb is heading right or left.
      const forward = target.left >= fromLeft
      const lead = forward ? rightInset : left
      const trail = forward ? left : rightInset
      const leadTarget = forward ? targetInset : target.left
      const trailTarget = forward ? target.left : targetInset

      const dilate = { duration: DILATE_MS / 1000, ease: EASE_OUT }
      animate(left, Math.min(fromLeft, target.left), dilate)
      animate(rightInset, Math.min(fromInset, targetInset), dilate)

      await new Promise(resolve => setTimeout(resolve, CONTRACT_DELAY_MS))

      if (!alive()) {
        return
      }

      animate(lead, leadTarget, { type: 'spring', duration: 0.3, bounce: 0 })
      // The trailing edge lands slightly past the slot, then relaxes back.
      await animate(trail, trailTarget + OVERSHOOT_PX, { type: 'spring', duration: 0.3, bounce: 0 })

      if (alive()) {
        animate(trail, trailTarget, { type: 'spring', duration: 0.16, bounce: 0 })
      }
    },
    [left, rightInset, width]
  )

  const measure = React.useCallback(() => {
    const inner = innerRef.current

    if (!inner) {
      return
    }

    slots.current = buttonRefs.current.map(b => ({
      left: b?.offsetLeft ?? 0,
      right: (b?.offsetLeft ?? 0) + (b?.offsetWidth ?? 0)
    }))
    width.set(inner.offsetWidth)
  }, [width])

  React.useLayoutEffect(() => {
    const inner = innerRef.current

    if (!inner) {
      return
    }

    measure()
    jumpTo(activeIndex)
    setReady(true)

    const observer = new ResizeObserver(() => {
      measure()
      jumpTo(activeIndex)
    })

    observer.observe(inner)

    return () => observer.disconnect()
    // Re-measure when the option set changes; `activeIndex` is re-applied by the effect below.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [measure, jumpTo, options.length])

  const first = React.useRef(true)

  React.useEffect(() => {
    if (first.current) {
      first.current = false

      return
    }

    const ours = pendingId.current === value
    pendingId.current = null

    if (ours && !reduceMotion) {
      void glideTo(activeIndex)
    } else {
      jumpTo(activeIndex)
    }
  }, [activeIndex, glideTo, jumpTo, reduceMotion, value])

  const select = (id: T) => {
    if (disabled) {
      return
    }

    if (id === value) {
      return
    }

    pendingId.current = id
    onChange(id)
  }

  const onKeyDown = (event: React.KeyboardEvent, index: number) => {
    const last = options.length - 1

    const next =
      event.key === 'ArrowRight' || event.key === 'ArrowDown'
        ? (index + 1) % options.length
        : event.key === 'ArrowLeft' || event.key === 'ArrowUp'
          ? (index - 1 + options.length) % options.length
          : event.key === 'Home'
            ? 0
            : event.key === 'End'
              ? last
              : -1

    if (next < 0) {
      return
    }

    event.preventDefault()
    select(options[next].id)
    buttonRefs.current[next]?.focus()
  }

  const onPointerDown = (event: React.PointerEvent<HTMLDivElement>) => {
    const slot = slots.current[activeIndex]
    const inner = innerRef.current

    if (disabled || event.button !== 0 || !slot || !inner) {
      return
    }

    const x = event.clientX - inner.getBoundingClientRect().left

    // Only the thumb itself is draggable; other slots are plain taps.
    if (x < slot.left || x > slot.right) {
      return
    }

    drag.current = {
      startX: event.clientX,
      startLeft: left.get(),
      lastX: event.clientX,
      lastT: event.timeStamp,
      velocity: 0,
      active: false,
      pointerId: event.pointerId
    }
  }

  const onPointerMove = (event: React.PointerEvent<HTMLDivElement>) => {
    const d = drag.current
    const inner = innerRef.current

    if (!d || !inner || d.pointerId !== event.pointerId) {
      return
    }

    const dx = event.clientX - d.startX

    if (!d.active) {
      if (Math.abs(dx) < DEADZONE_PX) {
        return
      }

      d.active = true
      runId.current += 1
      inner.setPointerCapture(event.pointerId)
    }

    const dt = event.timeStamp - d.lastT

    if (dt > 0) {
      d.velocity = (event.clientX - d.lastX) / dt
    }

    d.lastX = event.clientX
    d.lastT = event.timeStamp

    const thumbWidth = slots.current[activeIndex].right - slots.current[activeIndex].left
    const min = slots.current[0].left
    const max = slots.current[options.length - 1].right - thumbWidth
    const raw = d.startLeft + dx
    const pos = raw < min ? min - (min - raw) * RUBBER : raw > max ? max + (raw - max) * RUBBER : raw

    left.set(pos)
    rightInset.set(width.get() - pos - thumbWidth)
  }

  const endDrag = (event: React.PointerEvent<HTMLDivElement>) => {
    const d = drag.current
    drag.current = null

    if (!d?.active) {
      return
    }

    const thumbWidth = slots.current[activeIndex].right - slots.current[activeIndex].left
    const center = left.get() + thumbWidth / 2
    let nearest = 0
    let best = Infinity

    slots.current.forEach((s, i) => {
      const dist = Math.abs((s.left + s.right) / 2 - center)

      if (dist < best) {
        best = dist
        nearest = i
      }
    })

    if (Math.abs(d.velocity) > FLICK_PX_PER_MS) {
      nearest = Math.min(options.length - 1, Math.max(0, activeIndex + Math.sign(d.velocity)))
    }

    innerRef.current?.releasePointerCapture(event.pointerId)

    if (options[nearest].id === value) {
      void glideTo(activeIndex)
    } else {
      select(options[nearest].id)
    }
  }

  return (
    <div
      className={cn(
        'inline-flex w-fit rounded-sm bg-[color-mix(in_srgb,var(--ui-text-primary)_7%,transparent)] p-0.5',
        disabled && 'opacity-50',
        className
      )}
    >
      <div
        aria-disabled={disabled || undefined}
        className="relative min-w-0 flex-1 touch-pan-y select-none"
        onPointerCancel={endDrag}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={endDrag}
        ref={innerRef}
        role="radiogroup"
      >
        <div className="grid auto-cols-fr grid-flow-col gap-0.5">
          {options.map(({ id, label, icon: Icon }, index) => (
            <button
              aria-checked={value === id}
              className={cn(
                slotClass,
                'rounded-sm text-(--ui-text-secondary) transition-colors hover:text-foreground disabled:cursor-default disabled:hover:text-(--ui-text-secondary)'
              )}
              disabled={disabled}
              key={id}
              onClick={() => select(id)}
              onKeyDown={event => onKeyDown(event, index)}
              ref={node => {
                buttonRefs.current[index] = node
              }}
              role="radio"
              tabIndex={value === id ? 0 : -1}
              type="button"
            >
              {Icon && <Icon className="size-3" />}
              {label}
            </button>
          ))}
        </div>
        <motion.div
          aria-hidden
          className="pointer-events-none absolute inset-0 bg-(--dt-foreground) text-(--dt-background)"
          style={{ clipPath, visibility: ready ? 'visible' : 'hidden' }}
        >
          <div className="grid auto-cols-fr grid-flow-col gap-0.5">
            {options.map(({ id, label, icon: Icon }) => (
              <span className={slotClass} key={id}>
                {Icon && <Icon className="size-3" />}
                {label}
              </span>
            ))}
          </div>
        </motion.div>
      </div>
    </div>
  )
}
