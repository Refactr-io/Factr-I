import { useStore } from '@nanostores/react'
import { useNavigate } from 'react-router'

import { hudTargetSessionId } from '@/app/hud/handoff'
import {
  $dismissedPanes,
  $hiddenTreePanes,
  $layoutTree,
  closeTreePane,
  isPaneVisible,
  revealTreePane,
  togglePaneVisible,
  treePanesWithPrefix
} from '@/components/pane-shell/tree/store'
import { Button } from '@/components/ui/button'
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from '@/components/ui/dropdown-menu'
import { Tip } from '@/components/ui/tooltip'
import { useI18n } from '@/i18n'
import { triggerHaptic } from '@/lib/haptics'
import { FileDiff, Folder, Globe, ListDetails, MoreVertical, Terminal } from '@/lib/icons'
import { cn } from '@/lib/utils'
import { toggleHud } from '@/store/hud'
import { $fileBrowserOpen, setFileBrowserOpen } from '@/store/layout'
import { openBrowserTab } from '@/store/preview'
import { $reviewOpen, toggleReview } from '@/store/review'
import { $sessionRailOpen, toggleSessionRail } from '@/store/session-rail'

import { SETTINGS_ROUTE } from '../routes'

import { titlebarButtonClass } from './titlebar'
import type { TitlebarTool } from './titlebar-controls'

const PREVIEW_PREFIX = 'preview-tile:'
const ICON = { 'aria-hidden': true, size: 16, stroke: 1.5 } as const
const ON = 'bg-(--ui-control-active-background) text-foreground'
const OFF = 'text-(--ui-text-secondary)'

/** Browser tabs are dynamic preview tiles: the Browser is "open" when any of them is on screen. */
const visiblePreviews = () => treePanesWithPrefix(PREVIEW_PREFIX).filter(isPaneVisible)

/**
 * The pane cluster at the top of the chat column, in Claude's order: session rail, then terminal, changes,
 * browser, files. Each button toggles a pane that lives in the layout tree; the active pill reads the tree.
 */
export function usePaneClusterTools(): TitlebarTool[] {
  const { t } = useI18n()
  const railOpen = useStore($sessionRailOpen)

  // Re-render whenever the tree or the hidden/dismissed sets change; `isPaneVisible` reads them live.
  useStore($layoutTree)
  useStore($hiddenTreePanes)
  useStore($dismissedPanes)

  // Each card's own store is the truth for its toggle (the bindings keep the tree in step with it).
  // The terminal collapses (store stays true behind a sibling tab or a minimized zone), so it reads the tree.
  const terminal = isPaneVisible('terminal')
  const changes = useStore($reviewOpen)
  const files = useStore($fileBrowserOpen)
  const previews = visiblePreviews()
  const browser = previews.length > 0

  const tool = (id: string, label: string, active: boolean, icon: TitlebarTool['icon'], onSelect: () => void): TitlebarTool => ({
    active,
    className: cn(active ? ON : OFF),
    icon,
    id,
    label,
    onSelect: () => {
      triggerHaptic('tap')
      onSelect()
    },
    tour: `pane-${id}`
  })

  return [
    tool('session-rail', t.titlebar.paneSession, railOpen, <ListDetails {...ICON} />, toggleSessionRail),
    tool('terminal', t.titlebar.paneTerminal, terminal, <Terminal {...ICON} />, () => togglePaneVisible('terminal')),
    tool('changes', t.titlebar.paneChanges, changes, <FileDiff {...ICON} />, () => toggleReview()),
    tool('browser', t.titlebar.paneBrowser, browser, <Globe {...ICON} />, () =>
      browser ? previews.forEach(closeTreePane) : openBrowserTab()
    ),
    tool('files', t.titlebar.paneFiles, files, <Folder {...ICON} />, () => {
      if (files) {
        closeTreePane('files')
      } else {
        setFileBrowserOpen(true)
        revealTreePane('files')
      }
    })
  ]
}

/** The cluster's last button: app actions that used to crowd the top bar. */
export function PaneClusterMore() {
  const { t } = useI18n()
  const navigate = useNavigate()

  return (
    <DropdownMenu>
      <Tip label={t.titlebar.paneMore} placement="toolbar">
        <DropdownMenuTrigger asChild>
          <Button
            aria-label={t.titlebar.paneMore}
            className={cn(titlebarButtonClass, 'bg-transparent select-none', OFF)}
            data-tour="pane-more"
            onPointerDown={event => event.stopPropagation()}
            size="icon-titlebar"
            type="button"
            variant="ghost"
          >
            <MoreVertical {...ICON} />
          </Button>
        </DropdownMenuTrigger>
      </Tip>
      <DropdownMenuContent align="end" className="min-w-48">
        <DropdownMenuItem onSelect={() => toggleHud(hudTargetSessionId())}>{t.titlebar.enterHud}</DropdownMenuItem>
        <DropdownMenuItem onSelect={() => navigate(SETTINGS_ROUTE)}>{t.titlebar.layoutSettings}</DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
