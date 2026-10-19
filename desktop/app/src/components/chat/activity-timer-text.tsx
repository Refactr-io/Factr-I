import { cn } from '@/lib/utils'

import { formatElapsed } from './activity-timer'

interface ActivityTimerTextProps {
  seconds: number
  className?: string
}

export function ActivityTimerText({ seconds, className }: ActivityTimerTextProps) {
  return (
    <span
      aria-hidden="true"
      className={cn(
        'inline-block shrink-0 font-sans text-xs leading-4 text-(--ui-text-secondary) tabular-nums',
        className
      )}
      // Reserve the same slot before 3s and across seconds/minutes boundaries.
      style={{ inlineSize: '8ch', visibility: seconds < 3 ? 'hidden' : 'visible' }}
    >
      {formatElapsed(seconds)}
    </span>
  )
}
