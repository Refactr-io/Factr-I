import { useStore } from '@nanostores/react'
import { useCallback, useEffect, useMemo, useRef } from 'react'
import { Navigate, useLocation, useNavigate } from 'react-router'

import { codiconIcon } from '@/components/ui/codicon'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger
} from '@/components/ui/dropdown-menu'
import { KbdCombo } from '@/components/ui/kbd'
import { getFactrConfigDefaults, getFactrConfigRecord, saveFactrConfig } from '@/factr'
import { useI18n } from '@/i18n'
import { triggerHaptic } from '@/lib/haptics'
import {
  Archive,
  Bell,
  Cloud,
  Cpu,
  Download,
  Globe,
  Info,
  Keyboard,
  KeyRound,
  Plug,
  RefreshCw,
  Search,
  Settings2,
  Upload,
  Wrench
} from '@/lib/icons'
import { isEditableTarget } from '@/lib/keybinds/combo'
import { typeToFocusChar } from '@/lib/keybinds/composer-focus-keys'
import { $commandPaletteOpen, openCommandPalettePage } from '@/store/command-palette'
import { confirm } from '@/store/confirm'
import { $activeConnectionId } from '@/store/connections'
import { bindingsFor } from '@/store/keybinds'
import { $localModelsEnabled } from '@/store/local-models-flag'
import { notifyError } from '@/store/notifications'
import { $settingsScopeProfile } from '@/store/settings-scope'

import { useRouteEnumParam } from '../hooks/use-route-enum-param'
import { OverlayMain, OverlayNav, type OverlayNavGroup, OverlaySplitLayout } from '../overlays/overlay-split-layout'
import { OverlayView } from '../overlays/overlay-view'

import { AboutSettings } from './about-settings'
import { AppearanceSettings } from './appearance-settings'
import { ConfigSettings } from './config-settings'
import { SECTIONS } from './constants'
import { GatewaySettings } from './gateway-settings'
import { KeybindSettings } from './keybind-settings'
import { KEYS_VIEWS, KeysSettings, type KeysView } from './keys-settings'
import { movedSettingsTabRedirect } from './moved-tabs'
import { NotificationsSettings } from './notifications-settings'
import { SettingsBreadcrumbContext } from './primitives'
import { PROVIDER_VIEWS, ProvidersSettings, type ProviderView } from './providers-settings'
import { SessionsSettings } from './sessions-settings'
import { SettingsSubpageHeader } from './subpage-navigation'
import { resolveSettingsSubpage, settingsSubpageIcon, settingsSubpages } from './subpages'
import type { SettingsPageProps, SettingsView as SettingsViewId } from './types'
import { vaultOwnerKey, VaultSettings } from './vault-settings'

const SETTINGS_VIEWS: readonly SettingsViewId[] = [
  ...SECTIONS.map(s => `config:${s.id}` as SettingsViewId),
  'providers',
  'gateway',
  // Legacy alias: the Connections page merged into Gateways. Kept in the enum
  // so saved `?tab=connections` deep links still resolve (redirected below).
  'connections',
  'keybinds',
  'keys',
  'vault',
  'notifications',
  'sessions',
  'about'
]

// Rail order and headings. Destinations keep their ids; only where they sit changes.
const NAV_SECTIONS: readonly {
  id: string
  labelKey: 'groupGeneral' | 'groupAgent' | 'groupAccounts' | 'groupMore'
  views: readonly string[]
}[] = [
  {
    id: 'general',
    labelKey: 'groupGeneral',
    views: ['config:model', 'config:chat', 'config:appearance', 'notifications', 'keybinds']
  },
  {
    id: 'agent',
    labelKey: 'groupAgent',
    views: ['config:workspace', 'config:safety', 'config:browser', 'vault', 'config:memory', 'config:voice']
  },
  { id: 'accounts', labelKey: 'groupAccounts', views: ['providers', 'keys', 'gateway'] },
  { id: 'more', labelKey: 'groupMore', views: ['config:advanced', 'sessions', 'about'] }
]

const NAV_RANK = new Map(
  NAV_SECTIONS.flatMap((section, s) => section.views.map((view, i) => [view, s * 100 + i] as const))
)
const NAV_SECTION_KEY = new Map(
  NAV_SECTIONS.flatMap(section => section.views.map(view => [view, section.labelKey] as const))
)

export function SettingsView({ onClose, onConfigSaved, onMainModelChanged }: SettingsPageProps) {
  const scopeProfile = useStore($settingsScopeProfile)
  const activeConnectionId = useStore($activeConnectionId)
  const { t } = useI18n()
  const navigate = useNavigate()
  const { hash, pathname, search } = useLocation()

  // MCP and Plugins moved out of Settings into Capabilities. Keep old
  // `/settings?tab=mcp|plugins` deep links working — `useRouteEnumParam` would
  // silently coerce the unknown tab to the default view otherwise.
  useEffect(() => {
    const redirect = movedSettingsTabRedirect(search)

    if (redirect) {
      navigate(redirect, { replace: true })
    }
  }, [navigate, search])

  const [activeView] = useRouteEnumParam('tab', SETTINGS_VIEWS, 'config:model' as SettingsViewId)
  const params = new URLSearchParams(search)
  const requestedSubpage = params.get('page')
  const subpage = resolveSettingsSubpage(activeView, params)
  const needsSubpageRedirect = Boolean(subpage && subpage !== requestedSubpage)
  const subpageSearch = new URLSearchParams(search)

  if (subpage) {
    subpageSearch.set('page', subpage)
  }

  const openSettingsPage = useCallback(
    (view: SettingsViewId, page?: string) => {
      const next = new URLSearchParams(search)

      for (const key of [
        'page',
        'field',
        'setting',
        'key',
        'aux',
        'session',
        'kind',
        'label',
        'origin',
        'pview',
        'kview',
        'bview'
      ]) {
        next.delete(key)
      }

      next.set('tab', view)
      const destination = page ?? settingsSubpages(view)[0]?.id

      if (destination) {
        next.set('page', destination)
      }

      navigate({ hash, pathname, search: `?${next}` }, { replace: true })
    },
    [hash, navigate, pathname, search]
  )

  const setActiveView = useCallback((view: SettingsViewId) => openSettingsPage(view), [openSettingsPage])

  // Connections merged into the unified Gateways page: land old
  // `?tab=connections` routes/bookmarks there instead of a dead entry.
  useEffect(() => {
    if (activeView === 'connections') {
      setActiveView('gateway')
    }
  }, [activeView, setActiveView])
  // Providers subnav (Accounts vs API keys) lives in its own param so each
  // sub-view is deep-linkable and survives a refresh.
  const [providerView, setProviderView] = useRouteEnumParam<ProviderView>('pview', PROVIDER_VIEWS, 'accounts')
  const [keysView] = useRouteEnumParam<KeysView>('kview', KEYS_VIEWS, 'tools')

  // Jump to a section + its sub-view in one navigate. Two sequential setters
  // would each read the same stale `search` and the second would clobber the
  // first's `tab` — so the sub-view never opened on narrow screens.
  const openSubView = useCallback(
    (tab: SettingsViewId, param: string, value: string, fallback: string) => {
      const params = new URLSearchParams(search)

      for (const key of ['page', 'field', 'setting', 'key', 'aux', 'session', 'kind', 'label', 'origin']) {
        params.delete(key)
      }

      params.set('tab', tab)

      if (value === fallback) {
        params.delete(param)
      } else {
        params.set(param, value)
      }

      const qs = params.toString()
      navigate({ hash, pathname, search: qs ? `?${qs}` : '' }, { replace: true })
    },
    [hash, navigate, pathname, search]
  )

  const openProviderView = useCallback(
    (view: ProviderView) => openSubView('providers', 'pview', view, 'accounts'),
    [openSubView]
  )

  const openKeysView = useCallback((view: KeysView) => openSubView('keys', 'kview', view, 'tools'), [openSubView])

  const importInputRef = useRef<HTMLInputElement | null>(null)

  const exportConfig = async () => {
    try {
      const cfg = await getFactrConfigRecord()
      const blob = new Blob([JSON.stringify(cfg, null, 2)], { type: 'application/json' })
      const url = URL.createObjectURL(blob)
      const a = document.createElement('a')
      a.href = url
      a.download = 'factr-config.json'
      a.click()
      URL.revokeObjectURL(url)
      triggerHaptic('success')
    } catch (err) {
      notifyError(err, t.settings.exportFailed)
    }
  }

  const resetConfig = async () => {
    const ok = await confirm({
      confirmLabel: t.settings.resetToDefaults,
      destructive: true,
      title: t.settings.resetConfirm
    })

    if (!ok) {
      return
    }

    try {
      await saveFactrConfig(await getFactrConfigDefaults())
      triggerHaptic('success')
      onConfigSaved?.()
    } catch (err) {
      notifyError(err, t.settings.resetFailed)
    }
  }

  const navGroups: OverlayNavGroup[] = useMemo(
    () =>
      (
        [
          ...SECTIONS.flatMap(s => {
            const view = `config:${s.id}` as SettingsViewId

            const entry = {
              active: activeView === view,
              icon: s.icon,
              id: view,
              label: t.settings.sections[s.id] ?? s.label,
              onSelect: () => setActiveView(view)
            }

            // Credential Vault lives beside the Browser section: it feeds the
            // browser's model-blind vault fill, so the two are one mental unit.
            if (s.id === 'browser') {
              return [
                entry,
                {
                  active: activeView === 'vault',
                  icon: KeyRound,
                  id: 'vault',
                  label: t.settings.nav.vault,
                  onSelect: () => setActiveView('vault')
                }
              ]
            }

            return [entry]
          }),
          {
            active: activeView === 'notifications',
            icon: Bell,
            id: 'notifications',
            label: t.settings.nav.notifications,
            onSelect: () => setActiveView('notifications')
          },
          {
            active: activeView === 'providers',
            children: [
              {
                active: activeView === 'providers' && providerView === 'accounts',
                icon: codiconIcon('account'),
                id: 'pview:accounts',
                label: t.settings.nav.providerAccounts,
                onSelect: () => openProviderView('accounts')
              },
              {
                active: activeView === 'providers' && providerView === 'keys',
                icon: KeyRound,
                id: 'pview:keys',
                label: t.settings.nav.providerApiKeys,
                onSelect: () => openProviderView('keys')
              },
              {
                active: activeView === 'providers' && providerView === 'custom-endpoints',
                icon: Globe,
                id: 'pview:custom-endpoints',
                label: t.settings.nav.providerCustomEndpoints,
                onSelect: () => openProviderView('custom-endpoints')
              },
              // Local models ships behind the --local launch flag: no flag, no
              // nav entry (the pane itself also refuses to render, so a stale
              // ?pview=local deep link falls back to accounts-shaped emptiness
              // rather than a hidden feature).
              ...($localModelsEnabled.get()
                ? [
                    {
                      active: activeView === 'providers' && providerView === 'local',
                      icon: Cpu,
                      id: 'pview:local',
                      label: t.settings.nav.providerLocalModels,
                      onSelect: () => openProviderView('local')
                    }
                  ]
                : [])
            ],
            gapBefore: true,
            icon: Cloud,
            id: 'providers',
            label: t.settings.nav.providers,
            onSelect: () => setActiveView('providers')
          },
          {
            active: activeView === 'gateway',
            icon: Plug,
            id: 'gateway',
            label: t.settings.nav.gateway,
            onSelect: () => setActiveView('gateway')
          },
          {
            active: activeView === 'keybinds',
            icon: Keyboard,
            id: 'keybinds',
            label: t.settings.nav.keybinds,
            onSelect: () => setActiveView('keybinds')
          },
          {
            active: activeView === 'keys',
            children: [
              {
                active: activeView === 'keys' && keysView === 'tools',
                icon: Wrench,
                id: 'kview:tools',
                label: t.settings.nav.keysTools,
                onSelect: () => openKeysView('tools')
              },
              {
                active: activeView === 'keys' && keysView === 'settings',
                icon: Settings2,
                id: 'kview:settings',
                label: t.settings.nav.keysSettings,
                onSelect: () => openKeysView('settings')
              }
            ],
            icon: Wrench,
            id: 'keys',
            label: t.settings.nav.apiKeys,
            onSelect: () => setActiveView('keys')
          },
          {
            active: activeView === 'sessions',
            icon: Archive,
            id: 'sessions',
            label: t.settings.nav.archivedChats,
            onSelect: () => setActiveView('sessions')
          },
          {
            active: activeView === 'about',
            gapBefore: true,
            icon: Info,
            id: 'about',
            label: t.settings.nav.about,
            onSelect: () => setActiveView('about')
          }
        ] as OverlayNavGroup[]
      )
        .map(group => {
          const view = group.id as SettingsViewId
          const children = settingsSubpages(view)
          const placed = {
            gapBefore: false,
            section: NAV_SECTION_KEY.has(group.id) ? t.settings.nav[NAV_SECTION_KEY.get(group.id)!] : undefined
          }

          return children.length
            ? {
                ...placed,
                ...group,
                children: children.map(page => ({
                  active: group.active && subpage === page.id,
                  icon: settingsSubpageIcon(page, group.icon),
                  id: `${view}:${page.id}`,
                  label: t.settings.subpages[page.labelKey],
                  onSelect: () => openSettingsPage(view, page.id)
                }))
              }
            : { ...group, ...placed }
        })
        .sort((a, b) => (NAV_RANK.get(a.id) ?? 999) - (NAV_RANK.get(b.id) ?? 999)),
    [
      activeView,
      keysView,
      providerView,
      subpage,
      t,
      setActiveView,
      openProviderView,
      openKeysView,
      openSettingsPage,
      openSubView
    ]
  )

  const activeGroup = navGroups.find(group => group.active)
  const activeChild = activeGroup?.children?.find(child => child.active)

  // Type-to-search: printable keystrokes on the Settings surface (outside any
  // field) open the settings-scoped palette, seeded with the character — same
  // reflex as the chat surface's type-to-focus, pointed at search instead.
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if ($commandPaletteOpen.get() || isEditableTarget(event.target)) {
        return
      }

      const char = typeToFocusChar(event)

      if (char === null || char === ' ') {
        return
      }

      event.preventDefault()
      openCommandPalettePage('settings', char)
    }

    window.addEventListener('keydown', onKeyDown)

    return () => window.removeEventListener('keydown', onKeyDown)
  }, [])

  // Search lives at the top of the rail. It stays a button (not a real input) so
  // type-to-search above keeps owning the keystrokes; it opens the settings-scoped palette.
  const searchCombo = bindingsFor('nav.commandPalette')[0]

  const searchField = (
    <button
      className="mb-2 flex h-8 w-full items-center gap-2 rounded-lg border border-transparent bg-(--ui-bg-tertiary) pl-2.5 pr-1.5 text-left text-(--ui-text-secondary) transition-colors hover:text-foreground"
      onClick={() => {
        triggerHaptic('open')
        openCommandPalettePage('settings')
      }}
      type="button"
    >
      <Search className="size-4 shrink-0" />
      <span className="min-w-0 flex-1 truncate text-[length:var(--conversation-text-font-size)]">
        {t.settings.search.pill}
      </span>
      {searchCombo && <KbdCombo combo={searchCombo} size="sm" variant="flat" />}
    </button>
  )

  const navFooter = (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          className="flex h-8 w-full items-center gap-2 rounded-lg px-2 text-left text-[length:var(--conversation-text-font-size)] text-(--ui-text-secondary) transition-colors hover:bg-(--chrome-action-hover) hover:text-foreground data-[state=open]:bg-(--chrome-action-hover) data-[state=open]:text-foreground"
          type="button"
        >
          <Settings2 className="size-4 shrink-0 text-muted-foreground/90" />
          <span className="min-w-0 flex-1 truncate">{t.settings.nav.configMenu}</span>
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" side="top">
        <DropdownMenuItem onSelect={() => void exportConfig()}>
          <Download />
          {t.settings.exportConfig}
        </DropdownMenuItem>
        <DropdownMenuItem
          onSelect={() => {
            triggerHaptic('open')
            importInputRef.current?.click()
          }}
        >
          <Upload />
          {t.settings.importConfig}
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuItem
          className="text-destructive focus:text-destructive"
          onSelect={() => {
            triggerHaptic('warning')
            void resetConfig()
          }}
        >
          <RefreshCw />
          {t.settings.resetToDefaults}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  )

  const activeSettingsContent =
    activeView === 'config:appearance' ? (
      <AppearanceSettings subpage={subpage} />
    ) : activeView === 'about' ? (
      <AboutSettings subpage={subpage} />
    ) : activeView === 'gateway' || activeView === 'connections' ? (
      // 'connections' renders the unified page too so the frame before
      // the alias redirect lands doesn't flash the fallback view.
      <GatewaySettings subpage={subpage} />
    ) : activeView === 'keybinds' ? (
      <KeybindSettings subpage={subpage} />
    ) : activeView.startsWith('config:') ? (
      <ConfigSettings
        activeSectionId={activeView.slice('config:'.length)}
        importInputRef={importInputRef}
        onConfigSaved={onConfigSaved}
        onMainModelChanged={onMainModelChanged}
        subpage={subpage}
      />
    ) : activeView === 'providers' ? (
      <ProvidersSettings
        key={scopeProfile}
        onClose={onClose}
        onConfigSaved={onConfigSaved}
        onMainModelChanged={onMainModelChanged}
        onViewChange={setProviderView}
        view={providerView}
      />
    ) : activeView === 'keys' ? (
      <KeysSettings view={keysView} />
    ) : activeView === 'notifications' ? (
      <NotificationsSettings subpage={subpage} />
    ) : activeView === 'vault' ? (
      <VaultSettings key={vaultOwnerKey(activeConnectionId, scopeProfile)} subpage={subpage} />
    ) : (
      <SessionsSettings subpage={subpage} />
    )

  return (
    <OverlayView closeLabel={t.settings.closeSettings} onClose={onClose}>
      <OverlaySplitLayout>
        <OverlayNav childrenInRail={false} footer={navFooter} groups={navGroups} header={searchField} />

        <OverlayMain className="px-0 pb-0">
          <SettingsBreadcrumbContext.Provider value>
            {activeGroup && <SettingsSubpageHeader child={activeChild} group={activeGroup} />}
            {needsSubpageRedirect ? (
              <Navigate replace to={{ hash, pathname, search: `?${subpageSearch}` }} />
            ) : (
              activeSettingsContent
            )}
          </SettingsBreadcrumbContext.Provider>
        </OverlayMain>
      </OverlaySplitLayout>
    </OverlayView>
  )
}

export { SettingsView as SettingsPage }
