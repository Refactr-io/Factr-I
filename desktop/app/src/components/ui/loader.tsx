import type { ComponentProps } from 'react'

import { useElapsedSeconds } from '@/components/chat/activity-timer'
import { ActivityTimerText } from '@/components/chat/activity-timer-text'
import { usePaneVisible } from '@/components/pane-shell/pane-visibility'
import { cn } from '@/lib/utils'

import { LoaderMark, type LoaderVariant } from './loader-mark'

export const LOADER_TYPES = [
  'original-thinking',
  'thinking-five',
  'thinking-nine',
  'rose-orbit',
  'rose-curve',
  'rose-two',
  'rose-three',
  'rose-four',
  'lissajous-drift',
  'lemniscate-bloom',
  'hypotrochoid-loop',
  'three-petal-spiral',
  'four-petal-spiral',
  'five-petal-spiral',
  'six-petal-spiral',
  'butterfly-phase',
  'cardioid-glow',
  'cardioid-heart',
  'heart-wave',
  'spiral-search',
  'fourier-flow'
] as const

export type LoaderType = (typeof LOADER_TYPES)[number]

// Legacy SDK curve options remain accepted; every wait now uses the pixel grid.
interface LoaderProps extends Omit<ComponentProps<'span'>, 'children'> {
  type?: LoaderType
  pathSteps?: number
  strokeScale?: number
  ariaLabel?: string
  className?: string
  label?: string
  paused?: boolean
  /** Supply the real operation's elapsed time when it predates this mount. */
  seconds?: number
  variant?: LoaderVariant
}

/** The supplied 3x3 wavefront, shared by every inline wait. CSS owns all motion. */
export function Loader({
  ariaLabel,
  className,
  label,
  paused = false,
  seconds,
  variant = 'Drive',
  type: _type,
  pathSteps: _pathSteps,
  strokeScale: _strokeScale,
  ...props
}: LoaderProps) {
  const name = props['aria-label'] ?? ariaLabel ?? label
  const visible = usePaneVisible()
  const elapsed = useElapsedSeconds(Boolean(label) && seconds === undefined && !paused && visible)

  return (
    <span
      {...props}
      aria-hidden={props['aria-hidden'] ?? (name ? undefined : true)}
      aria-label={name}
      className={cn('loader', className)}
      data-paused={paused || !visible ? 'true' : undefined}
      data-slot="loader"
      data-variant={variant}
      role={props.role ?? (name ? 'status' : undefined)}
    >
      <LoaderMark paused={paused} variant={variant} />
      {label ? (
        <>
          <span className="loader__label">{label}</span>
          <ActivityTimerText seconds={seconds ?? elapsed} />
        </>
      ) : null}
    </span>
  )
}
