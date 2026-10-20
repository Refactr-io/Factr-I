import './loader.css'

import type { IconProps } from '@tabler/icons-react'
import { forwardRef } from 'react'

import { usePaneVisible } from '@/components/pane-shell/pane-visibility'
import { cn } from '@/lib/utils'

export type LoaderVariant = 'Dots' | 'Drive' | 'Orbit'

const ORBIT_ORDER = [0, 1, 2, 5, 8, 7, 6, 3]

/** SVG keeps the existing loading-icon contract; only the nine pixels animate. */
export const LoaderMark = forwardRef<SVGSVGElement, IconProps & { paused?: boolean; variant?: LoaderVariant }>(
  function LoaderMark({ className, size = 14, style, paused = false, variant = 'Drive', ...props }, ref) {
    const visible = usePaneVisible()

    return (
      <svg
        aria-hidden="true"
        {...props}
        className={cn('loader__mark', className)}
        data-paused={paused || !visible ? 'true' : undefined}
        height={size}
        ref={ref}
        stroke="none"
        style={{ ...style, animation: 'none' }}
        viewBox="0 0 14 14"
        width={size}
      >
        {Array.from({ length: 9 }, (_, i) => {
          const row = Math.floor(i / 3)
          const col = i % 3
          const step = ORBIT_ORDER.indexOf(i)
          const delay = variant === 'Orbit' ? step * 110 : (col + Math.abs(row - 1)) * 90

          return (
            <rect
              className="loader__pixel"
              fill="currentColor"
              height="4"
              key={i}
              rx={variant === 'Dots' ? 2 : 0.5}
              style={{
                animationDelay: `${delay}ms`,
                animationDuration: variant === 'Orbit' ? '950ms' : '650ms',
                animationName: variant === 'Orbit' && step < 0 ? 'none' : undefined
              }}
              width="4"
              x={col * 5}
              y={row * 5}
            />
          )
        })}
      </svg>
    )
  }
)
