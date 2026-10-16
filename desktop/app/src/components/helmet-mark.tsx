import { cn } from '@/lib/utils'

import { HELMET_PATH, HELMET_VIEWBOX } from './helmet-path'

// The helmet mascot drawn bare: no tile, no backing box, one tone. Forest Black on light surfaces, Mist Green on
// dark ones (both from the Refactr palette); a caller on a fixed surface passes its own text colour.
export function HelmetMark({ className, ...props }: React.ComponentProps<'svg'>) {
  return (
    <svg
      aria-hidden="true"
      className={cn('shrink-0 text-[#1D3023] dark:text-[#D5E6DC]', className)}
      fill="currentColor"
      viewBox={HELMET_VIEWBOX}
      {...props}
    >
      <path d={HELMET_PATH} fillRule="evenodd" />
    </svg>
  )
}
