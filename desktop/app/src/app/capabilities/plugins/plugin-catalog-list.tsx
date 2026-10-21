import { useStore } from '@nanostores/react'
import { useEffect, useState } from 'react'

import type { ProfileScope } from '@/api/client'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/i18n'
import { fetchPluginCatalogRows, type PluginCatalogRow } from '@/lib/plugin-catalog'
import { $agentPlugins } from '@/store/agent-plugins'
import { openCatalogPluginInstall } from '@/store/plugin-catalog-install'

type State = { rows: PluginCatalogRow[]; status: 'error' | 'loading' | 'ready' }

/** The curated plugin catalog the backend ships: a plain list with an Install action per entry. */
export function PluginCatalogList({ profile, scope }: { profile: ProfileScope; scope: null | string }) {
  const p = useI18n().t.skills.plugins
  const [state, setState] = useState<State>({ rows: [], status: 'loading' })
  const installed = useStore($agentPlugins)

  useEffect(() => {
    let live = true

    fetchPluginCatalogRows(profile).then(
      rows => live && setState({ rows, status: 'ready' }),
      () => live && setState({ rows: [], status: 'error' })
    )

    return () => {
      live = false
    }
  }, [profile])

  if (state.status !== 'ready') {
    return (
      <p className="px-1 py-2 text-[0.6875rem] text-(--ui-text-tertiary)">
        {state.status === 'error' ? p.catalogLoadFailed : p.catalogLoading}
      </p>
    )
  }

  if (state.rows.length === 0) {
    return <p className="px-1 py-2 text-[0.6875rem] text-(--ui-text-tertiary)">{p.catalogEmpty}</p>
  }

  return (
    <ul className="flex flex-col" data-testid="plugin-catalog-list">
      {state.rows.map(row => {
        // Installed state comes from the profile-scoped plugin list, the same match the install dialog uses.
        const local = installed.find(item => item.catalog_name === row.name || item.name === row.name)
        const updateAvailable = Boolean(local?.update_available)

        return (
          <li
            className="flex items-center gap-3 border-b border-(--ui-stroke-tertiary) py-2 last:border-b-0"
            data-testid={`catalog-row-${row.name}`}
            key={row.name}
          >
            <div className="min-w-0 flex-1">
              <div className="truncate text-[length:var(--conversation-text-font-size)] font-medium text-foreground">
                {row.title}
              </div>
              {row.description && (
                <div className="line-clamp-2 text-[length:var(--conversation-caption-font-size)] text-(--ui-text-tertiary)">
                  {row.description}
                </div>
              )}
            </div>
            <Button
              disabled={Boolean(local) && !updateAvailable}
              onClick={() => openCatalogPluginInstall(row, scope)}
              size="sm"
              type="button"
              variant="secondary"
            >
              {local ? (updateAvailable ? p.catalogUpdate : p.catalogInstalled) : p.catalogInstall}
            </Button>
          </li>
        )
      })}
    </ul>
  )
}
