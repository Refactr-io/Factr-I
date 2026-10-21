import { useEffect, useState } from 'react'

import { Button } from '@/components/ui/button'
import { useI18n } from '@/i18n'
import { fetchPluginCatalogRows, type PluginCatalogRow } from '@/lib/plugin-catalog'
import { openCatalogPluginInstall } from '@/store/plugin-catalog-install'

type State = { rows: PluginCatalogRow[]; status: 'error' | 'loading' | 'ready' }

/** The curated plugin catalog the backend ships: a plain list with an Install action per entry. */
export function PluginCatalogList({ scope }: { scope: null | string }) {
  const p = useI18n().t.skills.plugins
  const [state, setState] = useState<State>({ rows: [], status: 'loading' })

  useEffect(() => {
    let live = true

    fetchPluginCatalogRows().then(
      rows => live && setState({ rows, status: 'ready' }),
      () => live && setState({ rows: [], status: 'error' })
    )

    return () => {
      live = false
    }
  }, [])

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
      {state.rows.map(row => (
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
            disabled={row.installed && !row.updateAvailable}
            onClick={() => openCatalogPluginInstall(row, scope)}
            size="sm"
            type="button"
            variant="secondary"
          >
            {row.installed ? (row.updateAvailable ? p.catalogUpdate : p.catalogInstalled) : p.catalogInstall}
          </Button>
        </li>
      ))}
    </ul>
  )
}
