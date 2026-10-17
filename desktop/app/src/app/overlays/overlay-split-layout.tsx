import { Fragment, memo, type ReactNode, useEffect, useId, useLayoutEffect, useRef, useState } from 'react'

import { Button } from '@/components/ui/button'
import { DisclosureCaret } from '@/components/ui/disclosure-caret'
import { TabDropdown } from '@/components/ui/tab-dropdown'
import { useI18n } from '@/i18n'
import type { IconComponent } from '@/lib/icons'
import { cn } from '@/lib/utils'

import { PAGE_MAX_W } from '../layout-constants'

import { OVERLAY_TOP_CLEARANCE } from './overlay-view'

// The wide rail and the narrow dropdown swap at exactly the width where
// OverlaySplitLayout drops to a single column, so the rail never stacks.
const RAIL_HIDDEN = 'max-[47.5rem]:hidden'
const BAR_HIDDEN = 'hidden max-[47.5rem]:flex'

interface OverlaySplitLayoutProps {
  children: ReactNode
  className?: string
}

interface OverlaySidebarProps {
  children: ReactNode
  className?: string
}

interface OverlayMainProps {
  children: ReactNode
  className?: string
}

interface OverlayNavItemProps {
  active: boolean
  current?: boolean
  icon: IconComponent
  /** Stable identity for the row, used as its `data-tour` handle. */
  id?: string
  label: string
  // Renders as an indented child of another nav item: smaller icon and a
  // lighter active state so it never competes with the boxed parent item.
  nested?: boolean
  onClick: () => void
  trailing?: ReactNode
}

export function OverlaySplitLayout({ children, className }: OverlaySplitLayoutProps) {
  return (
    <div
      className={cn(
        // Narrow: one column, and pin rows to [nav-bar auto | main 1fr] — without
        // an explicit template the grid's default align-content:stretch splits the
        // height evenly across the two rows, shoving the content to mid-screen.
        'grid h-full min-h-0 flex-1 grid-cols-[14rem_minmax(0,1fr)] overflow-hidden bg-transparent max-[47.5rem]:grid-cols-1 max-[47.5rem]:grid-rows-[auto_minmax(0,1fr)]',
        className
      )}
    >
      {children}
    </div>
  )
}

export function OverlaySidebar({ children, className }: OverlaySidebarProps) {
  return (
    <aside
      className={cn(
        // The left links sit beside (not under) the floating close button, so
        // they ride up via the shorter shared OVERLAY_TOP_CLEARANCE (same line
        // as a Panel header) instead of main's taller X-clearance. The bg still
        // fills from the card's top edge, so there's no gap above the sidebar.
        'flex min-h-0 flex-col gap-0.5 overflow-y-auto bg-(--ui-bg-editor) px-3 pb-3 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden',
        OVERLAY_TOP_CLEARANCE,
        className
      )}
      // Every overlay's left nav (settings, cron, profiles, agents) answers to
      // one name, so a tour can point at "the nav" without knowing which
      // overlay is open. See lib/tour.
      data-tour="overlay-nav"
    >
      {children}
    </aside>
  )
}

export function OverlayMain({ children, className }: OverlayMainProps) {
  return (
    <main
      className={cn(
        // Main sits UNDER the floating close button (top-right), so it keeps the
        // taller top pad to clear the X — unlike the sidebar / Panel header,
        // which sit to its left and ride up via OVERLAY_TOP_CLEARANCE. All four
        // paddings are 1/3 tighter than the raw values (×2/3): the wide/narrow
        // top clearance, the bottom gutter, and the horizontal clamp gutter
        // (inlined from PAGE_INSET_X so only overlay panes tighten, not the
        // shared page gutter). Narrow top drops toward the OverlayNav bar.
        'mx-auto flex min-h-0 w-full flex-1 flex-col overflow-hidden bg-transparent pb-2 pt-[calc((var(--titlebar-height)/2+1rem)*2/3)] max-[47.5rem]:pt-[calc(0.5rem*2/3)] px-[clamp(0.8333rem,2.6667vw,2.6667rem)]',
        PAGE_MAX_W,
        className
      )}
    >
      {children}
    </main>
  )
}

export const OverlayNavItem = memo(function OverlayNavItem({
  active,
  current = active,
  icon: Icon,
  id,
  label,
  nested,
  onClick,
  trailing
}: OverlayNavItemProps) {
  return (
    <button
      aria-current={current ? 'page' : undefined}
      className={cn(
        'flex h-8 w-full items-center justify-start gap-2.5 rounded-md px-3 text-left text-[length:var(--conversation-text-font-size)] font-normal leading-[1.125rem] transition-colors',
        nested
          ? active
            ? 'bg-(--chrome-action-hover) font-medium text-foreground'
            : 'bg-transparent text-(--ui-text-tertiary) hover:bg-(--chrome-action-hover) hover:text-foreground'
          : active
            ? 'bg-(--ui-bg-tertiary) font-medium text-foreground'
            : 'bg-transparent text-(--ui-text-secondary) hover:bg-(--chrome-action-hover) hover:text-foreground'
      )}
      // Names the row by its own id, so a tour can address one link
      // (`[data-tour="nav-models"]`) instead of guessing at nth-child.
      data-tour={id ? `nav-${id}` : undefined}
      onClick={onClick}
      type="button"
    >
      <Icon
        className={cn(
          'shrink-0',
          nested ? 'size-3.5' : 'size-4',
          active ? 'text-foreground/80' : 'text-muted-foreground/90'
        )}
      />
      <span className="min-w-0 flex-1 truncate">{label}</span>
      {trailing}
    </button>
  )
})

/** A sub-section link under the selected group: text only, brighter and weight 500 on the current one. The rail
 *  lives in OverlayNavSubList. */
/** Inset of the marker inside a sub-item row (py-2 rows are 34px, so the marker is 22px, about 65% of the row). The
 *  track runs from the first marker position to the last, so the marker never overhangs it. */
const SUB_ITEM_PAD_Y = 6

function OverlayNavSubItem({ active, label, onClick }: { active: boolean; label: string; onClick: () => void }) {
  return (
    <button
      aria-current={active ? 'location' : undefined}
      className={cn(
        'w-full rounded-md py-2 pr-2 pl-0 text-left text-[length:var(--conversation-text-font-size)] leading-[1.125rem] transition-colors',
        active ? 'font-medium text-foreground' : 'text-(--ui-text-tertiary) hover:text-foreground'
      )}
      data-slot="overlay-nav-sub"
      onClick={onClick}
      type="button"
    >
      {label}
    </button>
  )
}

/** One continuous rail down the left of the sub-links with a single marker that slides to the current one
 *  (transform + height only; instant under reduced motion). */
function OverlayNavSubList({ children, className, hidden, id }: { children: ReactNode; className?: string; hidden: boolean; id: string }) {
  const listRef = useRef<HTMLDivElement>(null)
  const [mark, setMark] = useState<{ h: number; y: number } | null>(null)
  const [ready, setReady] = useState(false)

  useLayoutEffect(() => {
    const list = listRef.current
    const el = list?.querySelector<HTMLElement>('[aria-current="location"]')

    const next = el ? { h: el.offsetHeight - 2 * SUB_ITEM_PAD_Y, y: el.offsetTop + SUB_ITEM_PAD_Y } : null

    setMark(prev => (prev?.h === next?.h && prev?.y === next?.y ? prev : next))
  })

  useEffect(() => {
    const t = requestAnimationFrame(() => setReady(true))

    return () => cancelAnimationFrame(t)
  }, [])

  return (
    <div className={cn('relative', className, hidden && 'hidden')} hidden={hidden} id={id} ref={listRef}>
      <span aria-hidden className="pointer-events-none absolute top-1.5 bottom-1.5 left-px w-px rounded-full bg-[color-mix(in_srgb,var(--dt-foreground)_14%,transparent)]" />
      {mark && (
        <span
          aria-hidden
          className={cn(
            'pointer-events-none absolute top-0 left-0 w-[3px] rounded-full bg-foreground motion-reduce:transition-none',
            ready && 'transition-[transform,height] duration-[260ms] ease-[cubic-bezier(0.23,1,0.32,1)]'
          )}
          style={{ height: mark.h, transform: `translateY(${mark.y}px)` }}
        />
      )}
      {children}
    </div>
  )
}

export interface OverlayNavLink {
  active: boolean
  icon: IconComponent
  id: string
  label: string
  onSelect: () => void
}

export interface OverlayNavGroup extends OverlayNavLink {
  /** Sub-links: revealed by navigation or independently by the disclosure.
   *  Always listed (flattened + indented) in the narrow dropdown. */
  children?: OverlayNavLink[]
  /** Visual break before this group — a spacer on the rail, a separator in
   *  the dropdown. */
  gapBefore?: boolean
  /** Rail-only heading shown above the first group of each run that shares it. */
  section?: string
}

// Data-driven pane nav: one model renders a persistent left rail on wide
// viewports and a single dropdown bar on narrow ones (matching the tab
// dropdown in PageSearchShell), so every OverlaySplitLayout pane degrades the
// same way instead of stacking its whole sidebar. Drop it in as the first
// child of an OverlaySplitLayout, before OverlayMain.
export function OverlayNav({
  activeOnlyDisclosure = false,
  childrenInRail = true,
  footer,
  groups,
  header
}: {
  /** False keeps sub-links out of the wide rail (the page shows them instead); the narrow dropdown still lists them. */
  childrenInRail?: boolean
  /** Only the selected group shows its sub-items (a marker on the current one); there is no per-group toggle. */
  activeOnlyDisclosure?: boolean
  footer?: ReactNode
  groups: OverlayNavGroup[]
  header?: ReactNode
}) {
  const { t } = useI18n()
  const navId = useId()
  const [disclosures, setDisclosures] = useState<Record<string, boolean>>({})
  const activeGroup = groups.find(group => group.active)
  const activeGroupId = activeGroup?.id
  const activeChildId = activeGroup?.children?.find(child => child.active)?.id

  // Route entry reveals its branch. Explicitly opened inactive branches stay
  // open, while automatically revealed branches fold when leaving them.
  useEffect(() => {
    if (!activeGroupId) {
      return
    }

    setDisclosures(previous => {
      if (previous[activeGroupId] !== false) {
        return previous
      }

      const next = { ...previous }
      delete next[activeGroupId]

      return next
    })
  }, [activeGroupId, activeChildId])

  return (
    <>
      <OverlaySidebar className={RAIL_HIDDEN}>
        {header && <div className="sticky top-0 z-10 -mx-3 bg-(--ui-bg-editor) px-3 pt-1">{header}</div>}
        {groups.map((group, index) => {
          const hasChildren = childrenInRail && Boolean(group.children?.length)
          const sectionLabel = group.section && group.section !== groups[index - 1]?.section ? group.section : null
          const expanded = activeOnlyDisclosure ? group.active : (disclosures[group.id] ?? group.active)
          const childrenId = `${navId}-${group.id}`

          return (
            <Fragment key={group.id}>
              {sectionLabel ? (
                <div className="px-3 pb-1.5 pt-5 text-[length:var(--conversation-caption-font-size)] font-medium leading-4 text-foreground first:pt-1">
                  {sectionLabel}
                </div>
              ) : (
                group.gapBefore && <div aria-hidden className="h-2" />
              )}
              <div className="relative">
                <OverlayNavItem
                  active={group.active}
                  current={group.active && !group.children?.some(child => child.active)}
                  icon={group.icon}
                  id={group.id}
                  label={group.label}
                  onClick={() => {
                    if (hasChildren) {
                      setDisclosures(previous => {
                        if (previous[group.id] !== false) {
                          return previous
                        }

                        const next = { ...previous }
                        delete next[group.id]

                        return next
                      })
                    }

                    group.onSelect()
                  }}
                  trailing={
                    hasChildren && !activeOnlyDisclosure ? <span aria-hidden className="w-4 shrink-0" /> : undefined
                  }
                />
                {hasChildren && !activeOnlyDisclosure && (
                  <Button
                    aria-controls={childrenId}
                    aria-expanded={expanded}
                    aria-label={`${expanded ? t.common.collapse : t.common.expand}: ${group.label}`}
                    className="absolute right-0.5 top-1/2 -translate-y-1/2"
                    data-tour={`nav-toggle-${group.id}`}
                    onClick={() => setDisclosures(previous => ({ ...previous, [group.id]: !expanded }))}
                    size="icon-xs"
                    type="button"
                    variant="ghost"
                  >
                    <DisclosureCaret open={expanded} />
                  </Button>
                )}
              </div>
              {hasChildren && (
                <OverlayNavSubList
                  className={activeOnlyDisclosure ? 'mt-1 ml-[1.375rem] flex flex-col gap-0.5 pl-[1.125rem]' : 'ml-4 flex flex-col gap-0.5 pl-2'}
                  hidden={!expanded}
                  id={childrenId}
                >
                  {group.children?.map(child =>
                    activeOnlyDisclosure ? (
                      <OverlayNavSubItem
                        active={child.active}
                        key={child.id}
                        label={child.label}
                        onClick={child.onSelect}
                      />
                    ) : (
                      <OverlayNavItem
                        active={child.active}
                        icon={child.icon}
                        id={child.id}
                        key={child.id}
                        label={child.label}
                        nested
                        onClick={child.onSelect}
                      />
                    )
                  )}
                </OverlayNavSubList>
              )}
            </Fragment>
          )
        })}
        {footer && <div className="mt-auto flex items-center gap-1 pt-2">{footer}</div>}
      </OverlaySidebar>

      {/* Narrow: ride the OverlayView titlebar strip so the dropdown shares the
          close button's row instead of taking its own. The bar is
          pointer-events-none (children opt back in) so the floating X underneath
          stays clickable; pr clears it, no-drag beats the strip's drag region,
          and the height matches the strip so the trigger lines up with the X. */}
      <div
        className={cn(
          'pointer-events-none relative z-20 h-[calc(var(--titlebar-height)+0.1875rem)] items-center justify-between gap-2 pl-3 pr-12',
          BAR_HIDDEN
        )}
      >
        <div className="pointer-events-auto min-w-0 [-webkit-app-region:no-drag]">
          <TabDropdown
            align="start"
            items={groups.flatMap(group => [
              {
                active: group.active && !group.children?.some(child => child.active),
                icon: group.icon,
                id: group.id,
                label: group.label,
                onSelect: group.onSelect,
                separatorBefore: group.gapBefore
              },
              ...(group.children ?? []).map(child => ({
                active: child.active,
                icon: child.icon,
                id: child.id,
                indent: true,
                label: child.label,
                onSelect: child.onSelect
              }))
            ])}
          />
        </div>
        {footer && (
          <div className="pointer-events-auto flex shrink-0 items-center gap-1 [-webkit-app-region:no-drag]">
            {footer}
          </div>
        )}
      </div>
    </>
  )
}
