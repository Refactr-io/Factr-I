import { useStore } from '@nanostores/react'
import { useEffect, useState } from 'react'
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
import { FileDiff, Folder, Globe, LayoutPanelLeft, LayoutPanelRight, MoreVertical, Terminal } from '@/lib/icons'
import { cn } from '@/lib/utils'
import { toggleHud } from '@/store/hud'
import { $panesFlipped, setFileBrowserOpen } from '@/store/layout'
import { openBrowserTab } from '@/store/preview'
import { $reviewOpen, closeReview, toggleReview } from '@/store/review'
import { $sessionRailOpen, toggleSessionRail } from '@/store/session-rail'

import { SETTINGS_ROUTE } from '../routes'

import { titlebarButtonClass } from './titlebar'
import type { TitlebarTool } from './titlebar-controls'

const PREVIEW_PREFIX = 'preview-tile:'
const ICON = { 'aria-hidden': true, size: 16, stroke: 1.5 } as const
const ON = 'bg-(--ui-control-active-background) text-foreground'
const OFF = 'text-(--ui-text-secondary)'

/** The summary opens over the chat and shrinks to fit it; below this column width there is nothing to open it over. */
const RAIL_MIN_CHAT_PX = 288

/** Width of the chat column, from the edges the pane shell publishes. */
function useChatColumnWidth(): number {
  const read = () => {
    const css = getComputedStyle(document.documentElement)

    return window.innerWidth - (parseFloat(css.getPropertyValue('--workspace-left')) || 0) - (parseFloat(css.getPropertyValue('--workspace-right')) || 0)
  }

  const [width, setWidth] = useState(read)

  useEffect(() => {
    const update = () => setWidth(read())
    const id = window.setInterval(update, 400)

    window.addEventListener('resize', update)

    return () => {
      window.clearInterval(id)
      window.removeEventListener('resize', update)
    }
  }, [])

  return width
}

/** Browser tabs are dynamic preview tiles: the Browser is "open" when any of them is on screen. */
const visiblePreviews = () => treePanesWithPrefix(PREVIEW_PREFIX).filter(isPaneVisible)

/**
 * The pane cluster at the top of the chat column, in Claude's order: session rail, then terminal, changes,
 * browser, files. Each button toggles a pane that lives in the layout tree; the active pill reads the tree.
 */
export function usePaneClusterTools(): TitlebarTool[] {
  const { t } = useI18n()
  const railOpen = useStore($sessionRailOpen)
  const flipped = useStore($panesFlipped)
  const chatWidth = useChatColumnWidth()

  // Re-render whenever the tree or the hidden/dismissed sets change; `isPaneVisible` reads them live.
  useStore($layoutTree)
  useStore($hiddenTreePanes)
  useStore($dismissedPanes)

  // What is on screen is the truth for every pill: a store can read open while its card was dismissed or never
  // placed (a layout that rests it), and the button must never claim a card that is not there.
  const terminal = isPaneVisible('terminal')
  const changes = isPaneVisible('review')
  const files = isPaneVisible('files')
  const previews = visiblePreviews()


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
    // Greyed out only when the chat column is too narrow to open it over (an open one can always be closed).
    { ...tool('session-rail', t.titlebar.paneSession, railOpen, (flipped ? <LayoutPanelLeft {...ICON} /> : <LayoutPanelRight {...ICON} />), toggleSessionRail), disabled: !railOpen && chatWidth < RAIL_MIN_CHAT_PX },
    tool('terminal', t.titlebar.paneTerminal, terminal, <Terminal {...ICON} />, () => togglePaneVisible('terminal')),
    tool('changes', t.titlebar.paneChanges, changes, <FileDiff {...ICON} />, () => {
      if (changes) {
        closeReview()
        closeTreePane('review')
      } else {
        if ($reviewOpen.get()) {
          closeReview()
        }

        toggleReview()
        revealTreePane('review')
      }
    }),
    tool('browser', t.titlebar.paneBrowser, browser, <Globe {...ICON} />, () =>
      browser ? previews.forEach(closeTreePane) : openBrowserTab()
    ),
    tool('files', t.titlebar.paneFiles, files, <Folder {...ICON} />, () => {
      if (files) {
        // Close through the store too: closing only the card left the flag open, and every later layout read
        // "Files is open" for a card that was gone.
        setFileBrowserOpen(false)
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
        <DropdownMenuItem onSelect={() => navigate(`${SETTINGS_ROUTE}?tab=config:appearance&page=window-layout`)}>{t.titlebar.layoutSettings}</DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
