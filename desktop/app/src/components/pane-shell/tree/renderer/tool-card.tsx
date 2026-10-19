import { useStore } from '@nanostores/react'
import { type ReactNode, useEffect } from 'react'

import { Tip } from '@/components/ui/tooltip'
import { useI18n } from '@/i18n'
import { ArrowsMaximize, ArrowsMinimize, BottomPanel, PanelLeftIcon, PanelRightIcon, PanelTop, X } from '@/lib/icons'
import { cn } from '@/lib/utils'

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
  'grid size-6 shrink-0 place-items-center rounded-md text-(--ui-text-tertiary) transition-colors duration-100 ease-out [-webkit-app-region:no-drag] hover:bg-(--ui-control-hover-background) hover:text-foreground focus-visible:bg-(--ui-control-hover-background) focus-visible:text-foreground'

/** The card itself: a hairline-bordered squircle surface that floats in the zone's gutter. */
export function ToolCardShell({ children }: { children: ReactNode }) {
  return (
    <div
      className="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden rounded-lg border border-(--ui-stroke-secondary) bg-(--ui-bg-elevated)"
      data-tool-card=""
    >
      {children}
    </div>
  )
}

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
      <div className="group/arrange relative">
        <button
          aria-label={maximized ? c.restore : c.fill}
          className={iconButton}
          onClick={() => toggleMaximizedPane(paneId)}
          type="button"
        >
          {maximized ? <ArrowsMinimize aria-hidden size={14} stroke={1.5} /> : <ArrowsMaximize aria-hidden size={14} stroke={1.5} />}
        </button>
        <div
          className="absolute top-full right-0 z-60 hidden pt-1 group-focus-within/arrange:block group-hover/arrange:block"
          data-tool-card-menu=""
        >
          <div className="w-44 rounded-lg border border-(--ui-stroke-secondary) bg-popover p-2 text-xs text-(--ui-text-secondary)">
            <div className="px-1 pb-1 text-[0.6875rem] text-(--ui-text-tertiary)">{c.move}</div>
            <div className="flex gap-1 pb-1">
              {DOCKS.map(({ icon: Icon, key, pos }) => (
                <Tip key={pos} label={c[key]}>
                  <button aria-label={c[key]} className={iconButton} onClick={() => dockAround(pos)} type="button">
                    <Icon aria-hidden size={15} stroke={1.5} />
                  </button>
                </Tip>
              ))}
            </div>
            <button
              className={cn(iconButton, 'w-full justify-start gap-2 px-1.5 text-xs')}
              onClick={() => toggleMaximizedPane(paneId)}
              type="button"
            >
              {maximized ? <ArrowsMinimize aria-hidden size={14} stroke={1.5} /> : <ArrowsMaximize aria-hidden size={14} stroke={1.5} />}
              <span>{maximized ? c.restore : c.fill}</span>
            </button>
          </div>
        </div>
      </div>
      <Tip label={c.close}>
        <button aria-label={c.close} className={iconButton} onClick={onClose} type="button">
          <X aria-hidden size={14} stroke={1.5} />
        </button>
      </Tip>
    </div>
  )
}
