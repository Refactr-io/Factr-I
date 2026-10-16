import { cn } from '@/lib/utils'

import { HELMET_OUTER_PATH, HELMET_PATH, HELMET_VIEWBOX } from './helmet-path'

export type HelmetTone = 'auto' | 'dark' | 'light'

// The helmet mascot drawn bare: no tile, no backing box. The art is always Blackthorn Ink (#151313). On a dark
// surface a Rose Alloy (#FFFDFC) silhouette sits underneath it, with a 40-unit stroke that rings the outline, so the
// helmet reads like the black outline art on white and never merges with a dark ground; the panels and the star eyes
// show that light fill. `auto` follows the theme with plain CSS (no script, no flash); a mark on a surface that is
// always dark passes tone="dark", one that is always light tone="light".
export function HelmetMark({ className, tone = 'auto', ...props }: React.ComponentProps<'svg'> & { tone?: HelmetTone }) {
  return (
    <svg aria-hidden="true" className={cn('shrink-0', className)} viewBox={HELMET_VIEWBOX} {...props}>
      <path
        className={tone === 'dark' ? 'block' : tone === 'light' ? 'hidden' : 'hidden dark:block'}
        d={HELMET_OUTER_PATH}
        data-layer="light"
        fill="#FFFDFC"
        stroke="#FFFDFC"
        strokeLinejoin="round"
        strokeWidth={40}
      />
      <path d={HELMET_PATH} data-layer="ink" fill="#151313" fillRule="evenodd" />
    </svg>
  )
}
