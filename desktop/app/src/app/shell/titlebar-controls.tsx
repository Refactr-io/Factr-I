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

import { hudTargetSessionId } from '@/app/hud/handoff'
import { toggleLayoutEditMode } from '@/components/pane-shell/edit-mode'
import { resetLayoutTree } from '@/components/pane-shell/tree/store'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Tip, TipKeybindLabel } from '@/components/ui/tooltip'
import { Slot } from '@/contrib/react/slot'
import { useContributions } from '@/contrib/react/use-contributions'
import { useI18n } from '@/i18n'
import { NEW_SESSION_TITLE, sessionTitle } from '@/lib/chat-runtime'
import { triggerHaptic } from '@/lib/haptics'
import { formatModifierToken } from '@/lib/keybinds/combo'
import { cn } from '@/lib/utils'
import { toggleHud } from '@/store/hud'
import { $interfaceMode, shownInMode, type Tiered } from '@/store/interface-mode'
import {
  $fileBrowserOpen,
  $panesFlipped,
  $sidebarOpen,
  toggleFileBrowserOpen,
  togglePanesFlipped,
  toggleSidebarOpen
} from '@/store/layout'
import { $sessionTitleOverrides } from '@/store/read-only-sessions'
import { $sessions, sessionMatchesStoredId } from '@/store/session'
import { $unreadSessionCount } from '@/store/session-dot-state'
import { $focusedSessionIsTile, $focusedStoredSessionId } from '@/store/session-states'
import { $titlebarAppActionsSide, TITLEBAR_FIXED_TOOLS } from '@/store/titlebar-app-actions'

import { appViewForPath, hidesFixedTitlebarClusters, isOverlayView } from '../routes'

import {
  TITLEBAR_CHROME_CHANGED_EVENT,
  TITLEBAR_HEIGHT,
  TITLEBAR_ICON_BADGE_SCALE,
  titlebarButtonClass,
  titlebarIconSizeCss,
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
  onOpenSettings: () => void
}

/**
 * The layout button's glyph. Morphs into its composite reset form — the
 * layout icon wearing a small counter-clockwise arrow badge ("layout, back
 * to how it was") — ONLY while the pointer is on the button AND ⌘/Ctrl is
 * held: hover gates via CSS (`group/tool` on the button), the modifier via
 * the window listener. Pressing the modifier elsewhere changes nothing.
 */
function LayoutGlyph({ modHeld }: { modHeld: boolean }) {
  return (
    <>
      <span className={cn('inline-flex', modHeld && 'group-hover/tool:hidden')}>
        <TitlebarIcon name="layout" />
      </span>
      <span className={cn('relative hidden', modHeld && 'group-hover/tool:inline-flex')}>
        <TitlebarIcon name="layout" />
        <span className="absolute -bottom-1 -right-1.5 grid place-items-center rounded-full bg-(--ui-bg-chrome) p-px">
          <TitlebarIcon className="-scale-x-100" name="refresh" size={titlebarIconSizeCss(TITLEBAR_ICON_BADGE_SCALE)} />
        </span>
      </span>
    </>
  )
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

/** Live ⌘/Ctrl tracking — mod-click affordances telegraph themselves (the
 *  layout button morphs into its reset form while the modifier is down). */
function useModifierHeld(): boolean {
  const [held, setHeld] = useState(false)

  useEffect(() => {
    const sync = (event: KeyboardEvent) => setHeld(event.metaKey || event.ctrlKey)
    const clear = () => setHeld(false)

    window.addEventListener('keydown', sync)
    window.addEventListener('keyup', sync)
    window.addEventListener('blur', clear)

    return () => {
      window.removeEventListener('keydown', sync)
      window.removeEventListener('keyup', sync)
      window.removeEventListener('blur', clear)
    }
  }, [])

  return held
}

export function TitlebarControls({ leftTools = [], tools = [], onOpenSettings }: TitlebarControlsProps) {
  const { t } = useI18n()
  const navigate = useNavigate()
  const location = useLocation()
  const modHeld = useModifierHeld()
  const fileBrowserOpen = useStore($fileBrowserOpen)
  const panesFlipped = useStore($panesFlipped)
  const sidebarOpen = useStore($sidebarOpen)
  const unreadCount = useStore($unreadSessionCount)
  const appActionsSide = useStore($titlebarAppActionsSide)
  const interfaceMode = useStore($interfaceMode)
  const unreadBadge = unreadCount > 0 ? unreadCount : undefined
  const unreadHint = unreadBadge ? ` · ${t.titlebar.unreadSessions(unreadBadge)}` : ''
  // One filter for every cluster: a tool's own `hidden`, then the mode's tier.
  const shown = shownInMode(interfaceMode)
  const visibleTool = (tool: TitlebarTool) => !tool.hidden && shown(tool)

  // `titleBar.*` slot content is mount-scoped — a page's <Contribute> registers
  // only while that surface is up — so a non-empty area means a page is
  // actively projecting chrome into the band right now.
  const titleBarLeft = useContributions('titleBar.left')
  const titleBarRight = useContributions('titleBar.right')
  const pageOwnsTitlebar = titleBarLeft.length + titleBarRight.length > 0

  // POSITIONAL toggles: each button shows/hides everything on its physical
  // side of the main zone (the layout tree collapses the whole side), so they
  // stay correct through flips and rearranges. $sidebarOpen ≙ left side,
  // $fileBrowserOpen ≙ right side. Never an active highlight — plain
  // show/hide affordances.
  const leftEdge = { open: sidebarOpen, toggle: toggleSidebarOpen }
  const rightEdge = { open: fileBrowserOpen, toggle: toggleFileBrowserOpen }
  const leftLabel = leftEdge.open ? t.titlebar.hideSidebar : t.titlebar.showSidebar
  const rightLabel = rightEdge.open ? t.titlebar.hideRightSidebar : t.titlebar.showRightSidebar

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
      leftEdge.toggle()
    }
  }

  const flipTool: TitlebarTool = {
    ...TITLEBAR_FIXED_TOOLS['flip-panes'],
    actionId: 'view.flipPanes',
    icon: <TitlebarIcon name="arrow-swap" />,
    id: 'flip-panes',
    label: t.titlebar.swapSidebarSides,
    onSelect: () => {
      triggerHaptic('tap')
      togglePanesFlipped()
    }
  }

  const rightSidebarTool: TitlebarTool = {
    ...TITLEBAR_FIXED_TOOLS['right-sidebar'],
    actionId: 'view.toggleRightSidebar',
    badge: panesFlipped ? unreadBadge : undefined,
    icon: <TitlebarIcon name="layout-sidebar-right" />,
    id: 'right-sidebar',
    label: `${rightLabel}${panesFlipped ? unreadHint : ''}`,
    onSelect: () => {
      triggerHaptic('tap')
      rightEdge.toggle()
    },
    tour: 'right-pane-toggle'
  }

  // Static system tools — always pinned to the screen's right edge so the
  // left titlebar stays free for tabs (#107351).
  const systemTools: TitlebarTool[] = [
    {
      ...TITLEBAR_FIXED_TOOLS.settings,
      actionId: 'nav.settings',
      icon: <TitlebarIcon name="settings-gear" />,
      id: 'settings',
      label: t.titlebar.openSettings,
      onSelect: () => {
        triggerHaptic('open')
        onOpenSettings()
      }
    },
    {
      ...TITLEBAR_FIXED_TOOLS.layout,
      className: 'group/tool',
      // Hover + held ⌘/Ctrl morphs the glyph into its reset form (see
      // LayoutGlyph) — the mod-click telegraphs itself before it happens.
      icon: <LayoutGlyph modHeld={modHeld} />,
      id: 'layout',
      label: t.titlebar.layoutEditor,
      onSelect: event => {
        if (event?.metaKey || event?.ctrlKey) {
          triggerHaptic('warning')
          resetLayoutTree()

          return
        }

        triggerHaptic('open')
        toggleLayoutEditMode()
      },
      title: t.titlebar.layoutEditorTitle(formatModifierToken('mod'))
    },
    {
      ...TITLEBAR_FIXED_TOOLS.hud,
      // No `title`: TitlebarToolButton passes `title` to TipKeybindLabel as a
      // text OVERRIDE, so a long sentence there replaces the short label and
      // crowds the ⌘⇧H hint off the tooltip. Label only — the hint is appended
      // from the action registry, same as every other tool here.
      actionId: 'view.toggleHud',
      icon: <TitlebarIcon name="comment-discussion" />,
      id: 'hud',
      label: t.titlebar.enterHud,
      onSelect: () => {
        triggerHaptic('open')
        toggleHud(hudTargetSessionId())
      }
    }
  ]

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

  const visibleLeftTools = (
    appActionsSide === 'left'
      ? [backTool, forwardTool, sidebarTool, ...systemTools, ...leftTools]
      : [backTool, forwardTool, sidebarTool, ...leftTools]
  ).filter(visibleTool)

  const visibleSystemTools = appActionsSide === 'right' ? systemTools.filter(visibleTool) : []
  const visiblePaneTools = tools.filter(visibleTool)
  const visibleRightFixedTools = [flipTool, rightSidebarTool].filter(visibleTool)

  return (
    <>
      <TitlebarChatTitle />
      <TitlebarPartitionDashes />
      <div aria-label={t.shell.windowControls} className={leftClusterClass} data-titlebar-cluster="left">
        {visibleLeftTools.map(tool => (
          <TitlebarToolButton key={tool.id} navigate={navigate} tool={tool} />
        ))}
        <Slot area="titleBar.left" />
        <Slot area="titleBar.center" />
      </div>

      {visiblePaneTools.length > 0 && (
        <div
          aria-label={t.shell.appControls}
          className={cn(
            titlebarToolClusterClass,
            'top-[calc(var(--titlebar-controls-top)+var(--right-rail-top-inset,0px))] right-[calc(var(--titlebar-tools-right)+var(--shell-preview-toolbar-gap,0))]'
          )}
        >
          {visiblePaneTools.map(tool => (
            <TitlebarToolButton key={tool.id} navigate={navigate} tool={tool} />
          ))}
        </div>
      )}

      <div
        aria-label={t.shell.appControls}
        className={cn(titlebarToolClusterClass, 'right-(--titlebar-tools-right) top-(--titlebar-controls-top)')}
        data-titlebar-cluster="right"
      >
        {visibleSystemTools.map(tool => (
          <TitlebarToolButton key={tool.id} navigate={navigate} tool={tool} />
        ))}
        {visibleRightFixedTools.map(tool => (
          <TitlebarToolButton key={tool.id} navigate={navigate} tool={tool} />
        ))}
        <Slot area="titleBar.right" />
      </div>
    </>
  )
}

/** Keeps the titlebar chat title on the sessions/main partition, live. The
 *  sessions slot's right edge IS the partition, so a ResizeObserver on it tracks
 *  the fold transition frame by frame and sash drags with no timer of its own;
 *  the title also never starts inside the back/forward/toggle cluster, which
 *  slides in fullscreen (its own transition is followed frame by frame too). Values
 *  are written straight to the nodes: no React renders while it animates. */
function usePartitionTracker(ref: RefObject<HTMLDivElement | null>, enabled: boolean) {
  const sidebarOpen = useStore($sidebarOpen)

  useEffect(() => {
    const node = ref.current
    const slot = document.querySelector('[data-zone-panel]')?.closest('[data-tree-split] > div') as HTMLElement | null
    const cluster = document.querySelector('[data-titlebar-cluster="left"]') as HTMLElement | null

    if (!node || !enabled) {
      return
    }

    const title = node.querySelector('[data-titlebar-chat-title-text]') as HTMLElement | null

    const place = () => {
      // No sessions slot (flipped or hidden layouts): start at the block edge.
      // Unrounded: the sash hairline sits on the slot's exact (often fractional)
      // edge, and the divider must land on the same device pixels to read as
      // that line's top.
      const x = slot
        ? slot.getBoundingClientRect().right
        : (document.querySelector('[data-shell-block]')?.getBoundingClientRect().left ?? 0)

      node.style.left = `${x}px`

      if (title) {
        const clusterRight = cluster ? cluster.getBoundingClientRect().right : 0

        title.style.paddingLeft = `${Math.max(16, Math.round(clusterRight + 12 - x))}px`
      }
    }

    place()

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

    const observer = slot ? new ResizeObserver(place) : null

    if (slot) {
      observer?.observe(slot)
    }

    cluster?.addEventListener('transitionrun', start)
    cluster?.addEventListener('transitionend', stop)
    cluster?.addEventListener('transitioncancel', stop)
    window.addEventListener('resize', place)

    return () => {
      window.cancelAnimationFrame(frame)
      observer?.disconnect()
      cluster?.removeEventListener('transitionrun', start)
      cluster?.removeEventListener('transitionend', stop)
      cluster?.removeEventListener('transitioncancel', stop)
      window.removeEventListener('resize', place)
    }
  }, [enabled, sidebarOpen])
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

function TitlebarChatTitle() {
  const { pathname } = useLocation()
  const sessions = useStore($sessions)
  const focusedId = useStore($focusedStoredSessionId)
  const titleOverrides = useStore($sessionTitleOverrides)
  const focusedIsTile = useStore($focusedSessionIsTile)
  const ref = useRef<HTMLDivElement>(null)

  const visible = focusedIsTile || appViewForPath(pathname) === 'chat'

  usePartitionTracker(ref, visible)

  if (!visible) {
    return null
  }

  const session = focusedId ? sessions.find(s => sessionMatchesStoredId(s, focusedId)) : null
  // A session the list does not carry (a cron run) names itself from its resume reply.
  const title = session ? sessionTitle(session) : (focusedId && titleOverrides[focusedId]) || NEW_SESSION_TITLE

  return (
    <div
      className="pointer-events-none fixed top-0 z-70 flex items-center [-webkit-app-region:drag]"
      data-titlebar-chat-title=""
      ref={ref}
      style={{
        height: TITLEBAR_HEIGHT,
        right: 'calc(var(--titlebar-tools-right, 0px) + var(--titlebar-tools-width, 0px) + 0.75rem)'
      }}
    >
      <span
        className="min-w-0 translate-y-[0.5px] truncate pl-4 text-[0.8125rem] font-medium text-foreground"
        data-titlebar-chat-title-text=""
      >
        {title}
      </span>
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
