import { UnderlineTabs } from '@/components/ui/underline-tabs'
import { triggerHaptic } from '@/lib/haptics'
import { cn } from '@/lib/utils'

import { PAGE_INSET_X } from '../layout-constants'
import type { OverlayNavGroup, OverlayNavLink } from '../overlays/overlay-split-layout'

// Page title plus, when a page has sub-pages, underline tabs for them.
// Same onSelect handlers the rail used, so every deep link keeps resolving.
export function SettingsSubpageHeader({ group, child }: { group: OverlayNavGroup; child?: OverlayNavLink }) {
  const pages = group.children ?? []
  const current = child ?? pages[0]

  return (
    <header className={cn('mb-4 shrink-0', PAGE_INSET_X)}>
      <div className="mx-auto w-full max-w-[46rem]">
        <h1 className="text-xl font-semibold leading-7 tracking-[-0.01em] text-foreground">{group.label}</h1>
        {pages.length > 1 && (
          <UnderlineTabs
            ariaLabel={group.label}
            className="mt-3"
            onSelect={id => {
              const page = pages.find(p => p.id === id)

              if (page && page.id !== current?.id) {
                triggerHaptic('selection')
                page.onSelect()
              }
            }}
            tabs={pages.map(page => ({ id: page.id, label: page.label }))}
            value={current?.id ?? ''}
          />
        )}
      </div>
    </header>
  )
}
