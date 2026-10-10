import { triggerHaptic } from '@/lib/haptics'
import { cn } from '@/lib/utils'

import { PAGE_INSET_X } from '../layout-constants'
import type { OverlayNavGroup, OverlayNavLink } from '../overlays/overlay-split-layout'

// Page title plus, when a page has sub-pages, a segmented switch for them.
// Same onSelect handlers the rail used, so every deep link keeps resolving.
export function SettingsSubpageHeader({ group, child }: { group: OverlayNavGroup; child?: OverlayNavLink }) {
  const pages = group.children ?? []
  const current = child ?? pages[0]

  return (
    <header className={cn('mb-5 shrink-0', PAGE_INSET_X)}>
      <div className="mx-auto w-full max-w-[46rem]">
        <h1 className="text-xl font-semibold leading-7 tracking-[-0.01em] text-foreground">{group.label}</h1>
        {pages.length > 1 && (
          <div
            aria-label={group.label}
            className="no-scrollbar mt-3 inline-grid max-w-full auto-cols-fr grid-flow-col gap-0.5 overflow-x-auto rounded-lg bg-(--ui-bg-tertiary) p-0.5"
            role="tablist"
          >
            {pages.map(page => {
              const active = page.id === current?.id

              return (
                <button
                  aria-selected={active}
                  className={cn(
                    'h-7 whitespace-nowrap rounded-md px-4 text-[0.8125rem] transition-colors duration-150 motion-reduce:transition-none',
                    active
                      ? 'bg-(--ui-chat-surface-background) font-medium text-foreground shadow-[0_1px_1px_color-mix(in_srgb,var(--ui-text-primary)_10%,transparent)]'
                      : 'text-(--ui-text-secondary) hover:text-foreground'
                  )}
                  key={page.id}
                  onClick={() => {
                    if (!active) {
                      triggerHaptic('selection')
                      page.onSelect()
                    }
                  }}
                  role="tab"
                  type="button"
                >
                  {page.label}
                </button>
              )
            })}
          </div>
        )}
      </div>
    </header>
  )
}
