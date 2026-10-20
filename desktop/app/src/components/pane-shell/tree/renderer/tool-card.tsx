import { useStore } from '@nanostores/react'
import { type ReactNode, useEffect, useRef, useState } from 'react'

import { Popover, PopoverAnchor, PopoverContent } from '@/components/ui/popover'
import { Tip } from '@/components/ui/tooltip'
import { useI18n } from '@/i18n'
import { ArrowsMaximize, ArrowsMinimize, BottomPanel, PanelLeftIcon, PanelRightIcon, PanelTop, X } from '@/lib/icons'

import { allPaneIds, findGroupOfPane, isToolCardPane, type LayoutNode } from '../model'
import { $maximizedPane, restoreMaximizedPane, toggleMaximizedPane } from '../pane-maximize'
import { $layoutTree, moveTreePane } from '../store'

export { isToolCardPane }

/** A zone made only of tool cards: no seam is drawn beside it, the gutter and the card edge carry the boundary. */
export const isCardZone = (node: LayoutNode): boolean => {
  const ids = allPaneIds(node)

  return ids.length > 0 && ids.every(isToolCardPane)
}

const iconButton =
  'grid size-6 shrink-0 place-items-center rounded-sm text-(--ui-text-tertiary) transition-[color,background-color,scale] duration-150 ease-out active:scale-[0.96] motion-reduce:active:scale-100 [-webkit-app-region:no-drag] hover:bg-(--ui-control-hover-background) hover:text-foreground focus-visible:bg-(--ui-control-hover-background) focus-visible:text-foreground'

const menuButton =
  'grid size-7 shrink-0 place-items-center rounded-[calc(var(--radius-lg)-0.375rem)] text-(--ui-text-tertiary) transition-[color,background-color,scale] duration-150 ease-out active:scale-[0.96] motion-reduce:active:scale-100 hover:bg-(--ui-control-hover-background) hover:text-foreground focus-visible:bg-(--ui-control-hover-background) focus-visible:text-foreground'

/** The card itself: a squircle surface whose hairline is drawn above its content (a child can never cover the corner arc) that floats in the zone's gutter. */
export function ToolCardShell({ children }: { children: ReactNode }) {
  return (
    <div
      className="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden rounded-lg bg-(--ui-bg-elevated) after:pointer-events-none after:absolute after:inset-0 after:z-5 after:rounded-[inherit] after:border after:border-(--ui-stroke-secondary) after:content-['']"
      data-tool-card=""
    >
      {children}
    </div>
  )
}

/** When a card's Fill / Restore button last fired. Module-level: the card re-mounts as it fills. */
let lastToggleAt = 0

const DOCKS = [
  { icon: PanelLeftIcon, key: 'dockLeft', pos: 'left' },
  { icon: PanelRightIcon, key: 'dockRight', pos: 'right' },
  { icon: PanelTop, key: 'dockTop', pos: 'top' },
  { icon: BottomPanel, key: 'dockBottom', pos: 'bottom' }
] as const

/** Title on the left; arrange (hover: move around the chat, or fill the window) and close on the right. */
export function ToolCardHeader({ onClose, paneId, title }: { onClose: () => void; paneId: string; title: ReactNode }) {
  const { t } = useI18n()
  const c = t.titlebar.toolCard
  const maximized = useStore($maximizedPane) === paneId
  const [menuOpen, setMenuOpen] = useState(false)
  const closeTimer = useRef<number | undefined>(undefined)
  // After the button is used the pointer is still over it: stay shut until the pointer leaves and comes back.
  const justActed = useRef(false)

  // Hover (or focus) opens the arrange menu after a short intent delay, so a pointer passing by to another card does not
  // lay a menu over it; a grace period on the way out lets the pointer cross the gap into the menu.
  const openMenu = () => {
    window.clearTimeout(closeTimer.current)

    // The card re-mounts when it fills or restores, and the pointer is still over its new button: that is not a request
    // for the menu.
    if (!justActed.current && Date.now() - lastToggleAt > 700) {
      closeTimer.current = window.setTimeout(() => setMenuOpen(true), 220)
    }
  }

  const closeMenuSoon = () => {
    justActed.current = false
    window.clearTimeout(closeTimer.current)
    closeTimer.current = window.setTimeout(() => setMenuOpen(false), 140)
  }

  useEffect(() => () => window.clearTimeout(closeTimer.current), [])

  // Escape puts a filled pane back, except where a program inside the pane (the terminal) owns the key.
  useEffect(() => {
    if (!maximized) {
      return
    }

    const onKey = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null

      if (event.key === 'Escape' && !event.defaultPrevented && !target?.closest('.xterm, input, textarea, [contenteditable="true"]')) {
        restoreMaximizedPane()
      }
    }

    window.addEventListener('keydown', onKey)

    return () => window.removeEventListener('keydown', onKey)
  }, [maximized])

  const dockAround = (pos: (typeof DOCKS)[number]['pos']) => {
    const tree = $layoutTree.get()
    const main = tree ? findGroupOfPane(tree, 'workspace') : null

    if (main) {
      moveTreePane(paneId, { groupId: main.id, pos })
    }
  }

  return (
    <div
      className="flex h-9 shrink-0 items-center gap-1 pr-1.5 pl-3 select-none"
      data-tool-card-header=""
    >
      <span className="min-w-0 flex-1 truncate text-[0.8125rem] font-medium text-foreground">{title}</span>
      <Popover onOpenChange={setMenuOpen} open={menuOpen}>
        <PopoverAnchor asChild>
          <button
            aria-label={maximized ? c.restore : c.fill}
            className={iconButton}
            onBlur={closeMenuSoon}
            onClick={() => {
              lastToggleAt = Date.now()
              justActed.current = true
              window.clearTimeout(closeTimer.current)
              setMenuOpen(false)
              toggleMaximizedPane(paneId)
            }}
            onMouseEnter={openMenu}
            onMouseLeave={closeMenuSoon}
            type="button"
          >
            {maximized ? <ArrowsMinimize aria-hidden size={16} stroke={1.5} /> : <ArrowsMaximize aria-hidden size={16} stroke={1.5} />}
          </button>
        </PopoverAnchor>
        <PopoverContent
          align="end"
          className="w-40 p-1.5 text-xs text-(--ui-text-secondary)"
          collisionPadding={6}
          data-tool-card-menu=""
          onMouseEnter={openMenu}
          onMouseLeave={closeMenuSoon}
          onOpenAutoFocus={event => event.preventDefault()}
          sideOffset={6}
        >
          <div className="px-1.5 pt-0.5 pb-1.5 text-[0.6875rem] font-medium text-(--ui-text-tertiary)">{c.move}</div>
          <div className="flex gap-1 pb-1.5">
            {DOCKS.map(({ icon: Icon, key, pos }) => (
              <Tip key={pos} label={c[key]}>
                <button
                  aria-label={c[key]}
                  className={menuButton}
                  onClick={() => {
                    dockAround(pos)
                    setMenuOpen(false)
                  }}
                  type="button"
                >
                  <Icon aria-hidden size={16} stroke={1.5} />
                </button>
              </Tip>
            ))}
          </div>
          <button
            className="flex h-8 w-full items-center gap-2 rounded-[calc(var(--radius-lg)-0.375rem)] px-1.5 text-xs text-(--ui-text-secondary) transition-colors duration-100 ease-out hover:bg-(--ui-control-hover-background) hover:text-foreground focus-visible:bg-(--ui-control-hover-background) focus-visible:text-foreground"
            onClick={() => {
              toggleMaximizedPane(paneId)
              setMenuOpen(false)
            }}
            type="button"
          >
            {maximized ? <ArrowsMinimize aria-hidden size={16} stroke={1.5} /> : <ArrowsMaximize aria-hidden size={16} stroke={1.5} />}
            <span className="truncate">{maximized ? c.restore : c.fill}</span>
          </button>
        </PopoverContent>
      </Popover>
      <Tip label={c.close}>
        <button aria-label={c.close} className={iconButton} onClick={onClose} type="button">
          <X aria-hidden size={16} stroke={1.5} />
        </button>
      </Tip>
    </div>
  )
}
