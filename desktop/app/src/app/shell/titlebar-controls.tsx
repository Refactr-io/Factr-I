import { compactNumber } from '@factr/shared'
import { useStore } from '@nanostores/react'
import {
  type ComponentProps,
  type MouseEvent,
  type ReactNode,
  type RefObject,
  useEffect,
  useRef,
  useState
} from 'react'
import { useLocation, useNavigate } from 'react-router'

import { $maximizedPane } from '@/components/pane-shell/tree/pane-maximize'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Tip, TipKeybindLabel } from '@/components/ui/tooltip'
import { Slot } from '@/contrib/react/slot'
import { useContributions } from '@/contrib/react/use-contributions'
import { useI18n } from '@/i18n'
import { NEW_SESSION_TITLE, sessionTitle } from '@/lib/chat-runtime'
import { triggerHaptic } from '@/lib/haptics'
import { cn } from '@/lib/utils'
import { $interfaceMode, shownInMode, type Tiered } from '@/store/interface-mode'
import { $panesFlipped, $sidebarOpen, togglePanesFlipped, toggleSidebarOpen } from '@/store/layout'
import { $sessionTitleOverrides } from '@/store/read-only-sessions'
import { $sessions, sessionMatchesStoredId } from '@/store/session'
import { $unreadSessionCount } from '@/store/session-dot-state'
import { $focusedSessionIsTile, $focusedStoredSessionId } from '@/store/session-states'
import { TITLEBAR_FIXED_TOOLS } from '@/store/titlebar-app-actions'

import { appViewForPath, hidesFixedTitlebarClusters, isOverlayView } from '../routes'

import { PaneClusterMore, usePaneClusterTools } from './pane-cluster'
import { SessionRail } from './session-rail'
import {
  TITLEBAR_CHROME_CHANGED_EVENT,
  TITLEBAR_HEIGHT,
  titlebarButtonClass,
  titlebarToolClusterClass
} from './titlebar'
import { TitlebarIcon } from './titlebar-icon'
import { useNavHistory } from './use-nav-history'

export interface TitlebarTool extends Tiered {
  id: string
  label: string
  active?: boolean
  className?: string
  disabled?: boolean
  hidden?: boolean
  href?: string
  icon: ReactNode
  onSelect?: (event?: MouseEvent) => void
  /** Keybind action id — when set, the tooltip shows the label + keybind hint. */
  actionId?: string
  /** Overlay count on the glyph (unread sessions). Hidden when 0/undefined. */
  badge?: number
  title?: string
  to?: string
  /** Durable `data-tour` handle. Tools are addressed by icon and translated
   *  label otherwise, and neither survives a theme or a locale change. */
  tour?: string
}

export type TitlebarToolSide = 'left' | 'right'
export type SetTitlebarToolGroup = (id: string, tools: readonly TitlebarTool[], side?: TitlebarToolSide) => void

interface TitlebarControlsProps extends ComponentProps<'div'> {
  leftTools?: readonly TitlebarTool[]
  tools?: readonly TitlebarTool[]
  onOpenSettings?: () => void
}

/** Overlay count on a titlebar glyph. Hidden when count is 0/undefined. */
function withCountBadge(icon: ReactNode, count: number | undefined): ReactNode {
  if (!count) {
    return icon
  }

  return (
    <span className="relative inline-flex">
      {icon}
      <span className="pointer-events-none absolute -top-2.5 -right-1.5 z-1">
        <Badge aria-hidden size="overlay" variant="solid">
          {compactNumber(count)}
        </Badge>
      </span>
    </span>
  )
}

export function TitlebarControls({ leftTools = [], tools = [] }: TitlebarControlsProps) {
  const { t } = useI18n()
  const navigate = useNavigate()
  const location = useLocation()
  const panesFlipped = useStore($panesFlipped)
  const sidebarOpen = useStore($sidebarOpen)
  const unreadCount = useStore($unreadSessionCount)
  const interfaceMode = useStore($interfaceMode)
  const unreadBadge = unreadCount > 0 ? unreadCount : undefined
  const unreadHint = unreadBadge ? ` · ${t.titlebar.unreadSessions(unreadBadge)}` : ''
  // One filter for every cluster: a tool's own `hidden`, then the mode's tier.
  const shown = shownInMode(interfaceMode)
  const visibleTool = (tool: TitlebarTool) => !tool.hidden && shown(tool)
  const clusterTools = usePaneClusterTools()

  // `titleBar.*` slot content is mount-scoped — a page's <Contribute> registers
  // only while that surface is up — so a non-empty area means a page is
  // actively projecting chrome into the band right now.
  const titleBarLeft = useContributions('titleBar.left')
  const titleBarRight = useContributions('titleBar.right')
  const pageOwnsTitlebar = titleBarLeft.length + titleBarRight.length > 0

  const leftLabel = sidebarOpen ? t.titlebar.hideSidebar : t.titlebar.showSidebar
  const history = useNavHistory()

  const backTool: TitlebarTool = {
    disabled: !history.canGoBack,
    icon: <TitlebarIcon name="arrow-left" />,
    id: 'nav-back',
    label: t.titlebar.back,
    onSelect: () => history.back()
  }

  const forwardTool: TitlebarTool = {
    disabled: !history.canGoForward,
    icon: <TitlebarIcon name="arrow-right" />,
    id: 'nav-forward',
    label: t.titlebar.forward,
    onSelect: () => history.forward()
  }

  const sidebarTool: TitlebarTool = {
    ...TITLEBAR_FIXED_TOOLS.sidebar,
    actionId: 'view.toggleSidebar',
    badge: panesFlipped ? undefined : unreadBadge,
    icon: <TitlebarIcon name="layout-sidebar-left" />,
    id: 'sidebar',
    label: `${leftLabel}${panesFlipped ? '' : unreadHint}`,
    onSelect: () => {
      triggerHaptic('tap')
      toggleSidebarOpen()
    }
  }

  const swapTool: TitlebarTool = {
    icon: <TitlebarIcon name="arrow-swap" />,
    id: 'swap-sides',
    label: t.titlebar.swapSidebarSides,
    onSelect: () => {
      triggerHaptic('tap')
      togglePanesFlipped()
    },
    tour: 'swap-sides'
  }

  const view = appViewForPath(location.pathname)

  // Route changes can replace measured clusters without resizing the panels.
  useEffect(() => {
    window.dispatchEvent(new CustomEvent(TITLEBAR_CHROME_CHANGED_EVENT))
  }, [location.pathname, pageOwnsTitlebar])

  // Overlays own the window. These clusters are `fixed` at a higher z-index
  // than the overlay card, so they'd otherwise bleed over it — hide them (and
  // the nested titleBar slots) and let the overlay's own chrome take over.
  if (isOverlayView(view)) {
    return null
  }

  const leftClusterClass = cn(
    titlebarToolClusterClass,
    'left-(--titlebar-controls-left) top-(--titlebar-controls-top) translate-y-(--titlebar-controls-y-nudge)',
    // Entering/leaving macOS fullscreen slides the cluster between the
    // traffic-light inset and the window edge (same curve as the sidebar fold).
    'transition-[left] duration-[240ms] ease-[cubic-bezier(0.23,1,0.32,1)] motion-reduce:transition-none'
  )

  // A contributed full page (`extension`) yields the fixed clusters only while
  // it actually projects chrome into the band — page-mounted `titleBar.*` slots
  // like kanban's board switcher. A page that mounts no titlebar chrome keeps
  // the app's controls; an empty claim would leave a bare strip on every plugin
  // route. Contributed `titleBar.tools` items keep rendering here too, so a
  // chrome-owning page never silently drops a registered item.
  if (hidesFixedTitlebarClusters(view) && pageOwnsTitlebar) {
    const pageTools = [...leftTools, ...tools].filter(visibleTool)

    // Both markers are required even when a page contributes to only one side.
    return (
      <>
        <div className={leftClusterClass} data-titlebar-cluster="left">
          {pageTools.map(tool => (
            <TitlebarToolButton key={tool.id} navigate={navigate} tool={tool} />
          ))}
          <Slot area="titleBar.left" />
        </div>
        <div
          className={cn(titlebarToolClusterClass, 'right-(--titlebar-tools-right) top-(--titlebar-controls-top)')}
          data-titlebar-cluster="right"
        >
          <Slot area="titleBar.right" />
        </div>
      </>
    )
  }

  const visibleLeftTools = [backTool, forwardTool, sidebarTool, swapTool, ...leftTools].filter(visibleTool)
  const visiblePaneTools = tools.filter(visibleTool)

  return (
    <>
      <TitlebarPartitionDashes />
      <div aria-label={t.shell.windowControls} className={leftClusterClass} data-titlebar-cluster="left">
        {visibleLeftTools.map(tool => (
          <TitlebarToolButton key={tool.id} navigate={navigate} tool={tool} />
        ))}
        <Slot area="titleBar.left" />
        <Slot area="titleBar.center" />
      </div>

      <TitlebarChatHeader flipped={panesFlipped}>
        {visiblePaneTools.map(tool => (
          <TitlebarToolButton key={tool.id} navigate={navigate} tool={tool} />
        ))}
        {clusterTools.map(tool => (
          <TitlebarToolButton key={tool.id} navigate={navigate} tool={tool} />
        ))}
        <PaneClusterMore />
        <Slot area="titleBar.right" />
      </TitlebarChatHeader>
      <SessionRail />
    </>
  )
}

/** The active chat's title in the titlebar row, preceded by a 1px divider that
 *  continues the list/main partition upward. It belongs to the main column:
 *  it slides with the partition when the list folds or the sash drags. Chats
 *  switch from the sidebar; splits still open through the existing commands. */
/** One short dash in the titlebar over every vertical partition that reaches
 *  the top of the shell block (Codex): the list/chat seam, a right pane's seam,
 *  each on the exact pixel column of the line below it, centred in the bar and
 *  never touching the block. Seams are read from the sash hairlines themselves,
 *  so folds, drags and new panes are followed with no layout knowledge here. */
function TitlebarPartitionDashes() {
  const [xs, setXs] = useState<number[]>([])

  useEffect(() => {
    const block = document.querySelector('[data-shell-block]') as HTMLElement | null

    if (!block) {
      return
    }

    let frame = 0

    const measure = () => {
      frame = 0
      const box = block.getBoundingClientRect()

      const next = [...block.querySelectorAll<HTMLElement>('[data-sash-line]')]
        .map(line => line.getBoundingClientRect())
        .filter(r => r.width > 0 && r.width <= 2 && r.height > 0 && Math.abs(r.top - box.top) <= 2)
        .map(r => r.left)
        .filter(x => x > box.left + 4 && x < box.right - 4)
        .sort((a, b) => a - b)

      setXs(prev => (prev.length === next.length && prev.every((x, i) => x === next[i]) ? prev : next))
    }

    const schedule = () => {
      if (!frame) {
        frame = window.requestAnimationFrame(measure)
      }
    }

    const resize = new ResizeObserver(schedule)
    const watch = () => block.querySelectorAll('[data-tree-split] > div').forEach(slot => resize.observe(slot))

    const mutations = new MutationObserver(() => {
      watch()
      schedule()
    })

    resize.observe(block)
    watch()
    mutations.observe(block, { childList: true, subtree: true })
    window.addEventListener('resize', schedule)
    measure()

    return () => {
      window.cancelAnimationFrame(frame)
      resize.disconnect()
      mutations.disconnect()
      window.removeEventListener('resize', schedule)
    }
  }, [])

  return (
    <>
      {xs.map(x => (
        <span
          aria-hidden="true"
          className="pointer-events-none fixed z-70 h-4 w-px rounded-full bg-(--ui-stroke-tertiary)"
          data-titlebar-partition-dash=""
          key={x}
          style={{ left: x, top: (TITLEBAR_HEIGHT - 16) / 2 }}
        />
      ))}
    </>
  )
}

/** Keeps the header's left end clear of the left cluster (native window buttons, back, forward, sidebar toggle,
 *  swap). The chat column can start under that cluster - the sidebar folded, or sidebars swapped with the cluster of
 *  pane icons sitting at the column's left edge - so the header pads itself past it, whichever of the title and the
 *  pane icons is at the left. Everything else is pure CSS: the header is pinned to the main zone's edges. */
const PINNED_RIGHT = 'var(--workspace-right, 0px)'

function useLeftClearance(ref: RefObject<HTMLDivElement | null>, enabled: boolean): boolean {
  const sidebarOpen = useStore($sidebarOpen)
  const [crowded, setCrowded] = useState(false)

  useEffect(() => {
    const node = ref.current
    const cluster = document.querySelector('[data-titlebar-cluster="left"]') as HTMLElement | null

    if (!node || !cluster || !enabled) {
      return
    }

    const place = () => {
      const left = node.getBoundingClientRect().left
      const clearance = Math.round(cluster.getBoundingClientRect().right + 8 - left)

      node.style.paddingLeft = clearance > 0 ? `${clearance}px` : ''
      setCrowded(clearance > 0)

      // A narrow main zone (a filled Files card) can be tighter than clearance + the pane icons: let the bar run on
      // past its right edge instead of sliding the icons under the left controls.
      const icons = node.querySelector('[data-titlebar-cluster="right"]')?.getBoundingClientRect().width ?? 0
      const need = Math.max(clearance, 0) + icons

      node.style.right = PINNED_RIGHT
      node.style.width = ''
      const available = node.getBoundingClientRect().width

      if (available < need) {
        node.style.right = 'auto'
        node.style.width = `${need}px`
      }
    }

    place()
    const anchor = document.querySelector('[data-session-anchor="workspace"]')
    const observer = new ResizeObserver(place)

    if (anchor) {
      observer.observe(anchor)
    }

    let frame = 0

    const follow = () => {
      place()
      frame = window.requestAnimationFrame(follow)
    }

    const start = () => {
      window.cancelAnimationFrame(frame)
      follow()
    }

    const stop = () => {
      window.cancelAnimationFrame(frame)
      place()
    }

    // The chat zone's element is replaced when panes dock around it, so the observer above can end up on a node that
    // is gone; a slow poll and the layout stores keep the padding right whatever moved.
    const poll = window.setInterval(place, 200)

    cluster.addEventListener('transitionrun', start)
    cluster.addEventListener('transitionend', stop)
    cluster.addEventListener('transitioncancel', stop)
    window.addEventListener('resize', place)

    return () => {
      window.clearInterval(poll)
      node.style.paddingLeft = ''
      node.style.right = PINNED_RIGHT
      node.style.width = ''
      window.cancelAnimationFrame(frame)
      observer.disconnect()
      cluster.removeEventListener('transitionrun', start)
      cluster.removeEventListener('transitionend', stop)
      cluster.removeEventListener('transitioncancel', stop)
      window.removeEventListener('resize', place)
    }
  }, [enabled, ref, sidebarOpen])

  return crowded
}

/**
 * The chat column's header: the chat title and the pane cluster share one bar pinned to the main zone's edges
 * (`--workspace-left` / `--workspace-right`, published by the pane tree). Sidebars folding, panes opening and
 * sash drags move it with no per-frame measuring. Flipped layouts mirror the bar, so the cluster stays on the
 * side where the tool panes are and the title never lands on the wrong edge.
 */
function TitlebarChatHeader({ children, flipped }: { children: ReactNode; flipped: boolean }) {
  const { t } = useI18n()
  const { pathname } = useLocation()
  const sessions = useStore($sessions)
  const focusedId = useStore($focusedStoredSessionId)
  const titleOverrides = useStore($sessionTitleOverrides)
  const focusedIsTile = useStore($focusedSessionIsTile)
  const ref = useRef<HTMLDivElement>(null)
  const filled = useStore($maximizedPane) !== null

  // While a pane is in Fill the chat is out of the way, so its title is too.
  const visible = !filled && (focusedIsTile || appViewForPath(pathname) === 'chat')

  // A mirrored bar would park the pane icons against the left controls; when the chat starts under them, keep the
  // normal order (title left, icons at the chat's right edge).
  const crowded = useLeftClearance(ref, true)
  const mirrored = flipped && !crowded

  const session = focusedId ? sessions.find(s => sessionMatchesStoredId(s, focusedId)) : null
  // A session the list does not carry (a cron run) names itself from its resume reply.
  const title = session ? sessionTitle(session) : (focusedId && titleOverrides[focusedId]) || NEW_SESSION_TITLE

  return (
    <div
      className={cn(
        'pointer-events-none fixed top-0 z-70 flex items-center [-webkit-app-region:drag]',
        mirrored ? 'flex-row-reverse' : 'flex-row'
      )}
      data-titlebar-chat-header=""
      ref={ref}
      style={{ height: TITLEBAR_HEIGHT, left: 'var(--workspace-left, 0px)', right: PINNED_RIGHT }}
    >
      {visible ? (
        <span
          className={cn(
            'min-w-0 flex-1 truncate text-[0.8125rem] font-medium text-foreground',
            flipped ? 'pr-4 text-right' : 'pl-4'
          )}
          data-titlebar-chat-title=""
          data-titlebar-chat-title-text=""
        >
          {title}
        </span>
      ) : (
        <span className="flex-1" />
      )}
      <div
        aria-label={t.shell.appControls}
        className="pointer-events-auto flex shrink-0 flex-row items-center gap-0.5 px-2 select-none [-webkit-app-region:no-drag]"
        data-titlebar-cluster="right"
      >
        {children}
      </div>
    </div>
  )
}

function TitlebarToolButton({ navigate, tool }: { navigate: ReturnType<typeof useNavigate>; tool: TitlebarTool }) {
  // Titlebar actions never show an active background — state reads from the
  // icon itself (e.g. the mute/unmute glyph). aria-pressed still carries it
  // for a11y.
  const className = cn(titlebarButtonClass, 'bg-transparent select-none', tool.className)

  const tooltipLabel = tool.actionId ? (
    <TipKeybindLabel actionId={tool.actionId} text={tool.title ?? tool.label} />
  ) : (
    (tool.title ?? tool.label)
  )

  if (tool.href) {
    return (
      <Tip label={tooltipLabel} placement="toolbar">
        <Button asChild className={className} size="icon-titlebar" variant="ghost">
          <a
            aria-label={tool.label}
            data-tour={tool.tour}
            href={tool.href}
            onPointerDown={event => event.stopPropagation()}
            rel="noreferrer"
            target="_blank"
          >
            {withCountBadge(tool.icon, tool.badge)}
          </a>
        </Button>
      </Tip>
    )
  }

  return (
    <Tip label={tooltipLabel} placement="toolbar">
      <Button
        aria-label={tool.label}
        aria-pressed={tool.active ?? undefined}
        className={className}
        data-tour={tool.tour}
        disabled={tool.disabled}
        onClick={event => {
          if (tool.to) {
            navigate(tool.to)
          }

          tool.onSelect?.(event)
        }}
        onPointerDown={event => event.stopPropagation()}
        size="icon-titlebar"
        type="button"
        variant="ghost"
      >
        {withCountBadge(tool.icon, tool.badge)}
      </Button>
    </Tip>
  )
}
