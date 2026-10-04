import { useStore } from '@nanostores/react'
import { useLocation, useNavigate } from 'react-router'

import { findGroupOfPane } from '@/components/pane-shell/tree/model'
import { $layoutTree, activateTreePane } from '@/components/pane-shell/tree/store'
import { Codicon } from '@/components/ui/codicon'
import { Tip, TipKeybindLabel } from '@/components/ui/tooltip'
import { useContributions } from '@/contrib/react/use-contributions'
import { useI18n } from '@/i18n'
import { Command } from '@/lib/icons'
import { profileShortLabel } from '@/lib/profile-short-label'
import { cn } from '@/lib/utils'
import { SIDEBAR_RAIL_WIDTH } from '@/store/layout'
import { $selectedStoredSessionId } from '@/store/session'
import { $focusedSessionIsTile } from '@/store/session-states'

import { appViewForPath, navigateToWorkspacePage, NEW_CHAT_ROUTE, sessionRoute } from '../../routes'
import { useOverlayRouting } from '../../shell/hooks/use-overlay-routing'
import { TITLEBAR_HEIGHT } from '../../shell/titlebar'
import type { SidebarNavItem } from '../../types'

import { isNavItemActive } from './nav-active'
import { useSidebarNavItems } from './nav-model'
import { ProfileSwitcher } from './profile-dropdown-switcher'

const RAIL_BUTTON =
  'grid size-8 place-items-center rounded-md text-(--ui-text-secondary) transition-colors duration-100 ease-out [-webkit-app-region:no-drag] active:scale-[0.97] hover:bg-(--ui-control-hover-background) hover:text-foreground'

const RAIL_ACTIVE = 'bg-(--ui-control-active-background) text-foreground'

function RailButton({
  active,
  children,
  label,
  onClick,
  tip,
  tour
}: {
  active: boolean
  children: React.ReactNode
  label: string
  onClick: () => void
  tip?: React.ReactNode
  tour?: string
}) {
  return (
    <Tip label={tip ?? label} placement="left-rail">
      <button
        aria-current={active ? 'page' : undefined}
        aria-label={label}
        className={cn(RAIL_BUTTON, active && RAIL_ACTIVE)}
        data-tour={tour}
        onClick={onClick}
        type="button"
      >
        {children}
      </button>
    </Tip>
  )
}

/**
 * The sessions zone's icon rail (registered as `zoneChrome.rail`), on the window
 * surface. Always on screen: the zone folds down to exactly this column (⌘B).
 * Destinations are the zone's own panes (Sessions, Bots) and the primary pages,
 * each with its original icon; the active one is a filled rounded square. The
 * foot carries the profile avatar, which opens the existing profile switcher.
 */
export function SidebarRail() {
  const { t } = useI18n()
  const tree = useStore($layoutTree)
  const panesContrib = useContributions('panes')
  // The sessions zone's own panes (Sessions, Bots, ...) are rail destinations.
  const group = tree ? findGroupOfPane(tree, 'sessions') : null
  const activeId = group?.active ?? 'sessions'

  const panes = (group?.panes ?? ['sessions'])
    .map(id => panesContrib.find(c => c.id === id))
    .filter((c): c is NonNullable<typeof c> => Boolean(c))
    .map(c => {
      const data = c.data as undefined | { tabTitleText?: () => string }

      return { id: c.id, title: data?.tabTitleText?.() ?? c.title ?? c.id }
    })

  const activate = (paneId: string) => group && activateTreePane(group.id, paneId)
  const { pathname } = useLocation()
  const navigate = useNavigate()
  const items = useSidebarNavItems().filter(item => item.id !== 'new-session')
  // A focused tile is a chat, whatever route the main pane sits on.
  const focusedIsTile = useStore($focusedSessionIsTile)
  const view = focusedIsTile ? 'chat' : appViewForPath(pathname)
  const pageActive = items.some(item => isNavItemActive(item, view, pathname))
  const sessions = panes.find(pane => pane.id === 'sessions')
  const others = panes.filter(pane => pane.id !== 'sessions')
  const labelFor = (item: SidebarNavItem) => t.sidebar.nav[item.id] ?? item.label
  const { commandCenterOpen, toggleCommandCenter } = useOverlayRouting()
  const commandCenterLabel = commandCenterOpen
    ? t.shell.statusbar.closeCommandCenter
    : t.shell.statusbar.openCommandCenter

  const openSessions = () => {
    activate('sessions')

    if (view !== 'chat') {
      const id = $selectedStoredSessionId.get()

      navigateToWorkspacePage(navigate, id ? sessionRoute(id) : NEW_CHAT_ROUTE)
    }
  }

  return (
    <nav
      aria-label={t.sidebar.sessions}
      className="flex shrink-0 flex-col items-center gap-1 pb-1.5"
      data-sidebar-rail=""
      style={{ paddingTop: TITLEBAR_HEIGHT + 6, width: SIDEBAR_RAIL_WIDTH }}
    >
      {sessions && (
        <RailButton
          active={activeId === 'sessions' && !pageActive}
          label={sessions.title}
          onClick={openSessions}
          tour="rail-nav-sessions"
        >
          <Codicon name="home" size="1rem" />
        </RailButton>
      )}
      {items.map(item => {
        const label = labelFor(item)

        return (
          <RailButton
            active={isNavItemActive(item, view, pathname)}
            key={item.id}
            label={label}
            onClick={() => {
              if (item.route) {
                navigateToWorkspacePage(navigate, item.route)
              }
            }}
            tip={item.keybindActionId ? <TipKeybindLabel actionId={item.keybindActionId} text={label} /> : undefined}
            tour={`rail-nav-${item.id}`}
          >
            <item.icon className="size-4" />
          </RailButton>
        )
      })}
      {others.map(pane => (
        <RailButton
          active={activeId === pane.id && !pageActive}
          key={pane.id}
          label={pane.title}
          onClick={() => activate(pane.id)}
          tour={`rail-pane-${pane.id}`}
        >
          <Codicon name="robot" size="1rem" />
        </RailButton>
      ))}
      <div className="mt-auto flex flex-col items-center gap-1 pb-2">
        {/* The way into every other surface (was the status bar's ⌘). */}
        <RailButton
          active={commandCenterOpen}
          label={commandCenterLabel}
          onClick={toggleCommandCenter}
          tip={<TipKeybindLabel actionId="nav.commandCenter" text={commandCenterLabel} />}
          tour="rail-command-center"
        >
          <Command className="size-4" />
        </RailButton>
        <ProfileSwitcher
          contentAlign="end"
          contentSide="right"
          renderTrigger={({ label, name }) => (
            <button
              aria-label={label}
              className="grid size-7 place-items-center rounded-full bg-(--ui-control-active-background) text-xs font-medium text-foreground transition-colors duration-100 ease-out [-webkit-app-region:no-drag] hover:bg-(--ui-bg-tertiary) active:scale-[0.97]"
              type="button"
            >
              {profileShortLabel(name || 'default')}
            </button>
          )}
          wrapperClassName="inline-flex"
        />
      </div>
    </nav>
  )
}
