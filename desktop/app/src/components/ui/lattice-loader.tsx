import { Loader2 } from '@/lib/icons'
import { cn } from '@/lib/utils'

type LatticePattern = 'orbit' | 'ripple' | 'snake' | 'spiral'

interface LatticeLoaderProps {
  /** Omit when a neighbouring label already names what is loading. */
  ariaLabel?: string
  className?: string
  /** Freeze the mark (a surface fading out keeps a still one). */
  paused?: boolean
  /** Kept for existing callers; the loader is a plain spinner now (the dotted lattice grid is retired). */
  pattern?: LatticePattern
}

/** Small inline "loading" mark: a plain spinner, sized by the surrounding font size. */
export function LatticeLoader({ ariaLabel, className, paused = false }: LatticeLoaderProps) {
  return (
    <span
      aria-hidden={ariaLabel ? undefined : true}
      aria-label={ariaLabel}
      className={cn('inline-flex shrink-0 items-center justify-center', className)}
      role={ariaLabel ? 'status' : undefined}
    >
      <Loader2 className={cn('size-[1em]', paused ? '' : 'animate-spin')} stroke={1.75} />
    </span>
  )
}
