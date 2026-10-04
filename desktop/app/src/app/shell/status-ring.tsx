import { cn } from '@/lib/utils'

const SIZE = 14
const STROKE = 1.75
const RADIUS = (SIZE - STROKE) / 2
const CIRCUMFERENCE = 2 * Math.PI * RADIUS

/**
 * The composer row's status mark: a coloured ring for the gateway (green
 * ready, amber degraded, red offline), and while a turn runs or the gateway
 * connects, a short arc of the same colour spinning around it. Details (the
 * gateway state, the context window) are in its tooltip and popover.
 */
export function StatusRing({ busy, tone }: { busy: boolean; tone: 'error' | 'ready' | 'warn' }) {
  return (
    <svg
      aria-hidden="true"
      className={cn(
        'shrink-0',
        tone === 'ready' && 'text-[#2f8a4e] dark:text-[#62c083]',
        tone === 'warn' && 'text-amber-600',
        tone === 'error' && 'text-destructive'
      )}
      height={SIZE}
      viewBox={`0 0 ${SIZE} ${SIZE}`}
      width={SIZE}
    >
      <circle
        cx={SIZE / 2}
        cy={SIZE / 2}
        fill="none"
        r={RADIUS}
        stroke="currentColor"
        strokeOpacity={busy ? 0.3 : 1}
        strokeWidth={STROKE}
        style={{ transition: 'stroke-opacity 240ms cubic-bezier(0.23, 1, 0.32, 1)' }}
      />
      {busy ? (
        <circle
          className="origin-center animate-spin motion-reduce:animate-none"
          cx={SIZE / 2}
          cy={SIZE / 2}
          fill="none"
          r={RADIUS}
          stroke="currentColor"
          strokeDasharray={`${CIRCUMFERENCE * 0.28} ${CIRCUMFERENCE}`}
          strokeLinecap="round"
          strokeWidth={STROKE}
          style={{ animationDuration: '0.9s' }}
        />
      ) : null}
    </svg>
  )
}
