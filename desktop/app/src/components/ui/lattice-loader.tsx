import './lattice-loader.css'

import type { CSSProperties } from 'react'

import { usePaneVisible } from '@/components/pane-shell/pane-visibility'
import { cn } from '@/lib/utils'

// The grid of React Bits' Lattice Loader (reactbits.dev/micro/lattice-loader,
// MIT + Commons Clause), without its label and timer: it sits beside text the
// caller already shows. Timings, curves and patterns are theirs.
const STEP_MS = 90

type LatticePattern = 'orbit' | 'ripple' | 'snake' | 'spiral'

// Cell `n` lights `n` steps into the loop; `null` is a hole in the grid.
const PATTERNS: Record<LatticePattern, { cells: (number | null)[]; loop: number; scale: number; lit?: 35 }> = {
  orbit: { cells: [0, 1, 2, 7, null, 3, 6, 5, 4], loop: 8, scale: 1.2 },
  ripple: { cells: [2, 1, 2, 1, 0, 1, 2, 1, 2], loop: 4.8, scale: 1.5 },
  snake: { cells: [0, 1, 2, 5, 4, 3, 6, 7, 8], loop: 9, scale: 1, lit: 35 },
  spiral: { cells: [0, 1, 2, 7, 8, 3, 6, 5, 4], loop: 9, scale: 1.2, lit: 35 }
}

interface LatticeLoaderProps {
  /** Omit when a neighbouring label already names what is loading. */
  ariaLabel?: string
  className?: string
  /** Freeze the grid (a surface fading out keeps a still mark). */
  paused?: boolean
  pattern?: LatticePattern
}

/** Small inline "loading" mark: a 3×3 dot grid that lights in a loop. */
export function LatticeLoader({ ariaLabel, className, paused = false, pattern = 'orbit' }: LatticeLoaderProps) {
  const visible = usePaneVisible()
  const { cells, loop, scale, lit } = PATTERNS[pattern]
  const step = STEP_MS * scale

  return (
    <span
      aria-hidden={ariaLabel ? undefined : true}
      aria-label={ariaLabel}
      className={cn('lattice-loader', className)}
      data-paused={paused || !visible ? 'true' : undefined}
      role={ariaLabel ? 'status' : undefined}
      style={{ '--ll-cycle': `${Math.round(loop * step)}ms` } as CSSProperties}
    >
      {cells.map((unit, i) => (
        <span
          className="lattice-loader__cell"
          data-hole={unit === null ? '' : undefined}
          data-lit={lit}
          key={i}
          style={unit === null ? undefined : { animationDelay: `${Math.round(unit * step)}ms` }}
        />
      ))}
    </span>
  )
}
