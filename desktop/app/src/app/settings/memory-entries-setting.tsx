import { useCallback, useEffect, useState } from 'react'

import { factrApi, profileScoped } from '@/api/client'
import { Button } from '@/components/ui/button'
import { Tip } from '@/components/ui/tooltip'
import { useI18n } from '@/i18n'
import { Brain, Trash2 } from '@/lib/icons'
import { confirm } from '@/store/confirm'
import { notifyError } from '@/store/notifications'

import { ListRow, SectionHeading } from './primitives'

interface MemoryEntry {
  id: string
  content: string
  category: string
  source?: null | string
  active: boolean
}

const entriesPath = '/api/memory/entries'

/** What the engine has stored for the user, one row each, with Remove. Backends without the entries
 *  route (404) render nothing, so the page keeps working there. */
export function MemoryEntriesSetting() {
  const { t } = useI18n()
  const m = t.settings.config
  const [entries, setEntries] = useState<MemoryEntry[] | null>(null)
  const [supported, setSupported] = useState(true)

  const load = useCallback(async () => {
    try {
      const res = await factrApi<{ entries?: MemoryEntry[] }>({ ...profileScoped(), path: entriesPath })
      setEntries((res.entries ?? []).filter(entry => entry.active))
    } catch (err) {
      if (/404|not found|not supported/i.test(err instanceof Error ? err.message : String(err))) {
        setSupported(false)
      } else {
        notifyError(err, m.memoryEntriesFailed)
      }

      setEntries([])
    }
  }, [m.memoryEntriesFailed])

  useEffect(() => {
    void load()
  }, [load])

  const remove = async (entry: MemoryEntry) => {
    const ok = await confirm({
      confirmLabel: m.memoryRemove,
      description: entry.content,
      destructive: true,
      title: m.memoryRemoveConfirm
    })

    if (!ok) {
      return
    }

    const previous = entries
    setEntries(current => current?.filter(item => item.id !== entry.id) ?? current)

    try {
      await factrApi({ ...profileScoped(), path: `${entriesPath}/${encodeURIComponent(entry.id)}`, method: 'DELETE' })
    } catch (err) {
      setEntries(previous)
      notifyError(err, m.memoryRemoveFailed)
    }
  }

  if (!supported || entries === null) {
    return null
  }

  return (
    <section className="mb-4">
      <SectionHeading icon={Brain} meta={String(entries.length)} title={m.memoryEntriesTitle} />
      {entries.length === 0 ? (
        <p className="px-1 pb-2 text-[length:var(--conversation-caption-font-size)] text-(--ui-text-tertiary)">
          {m.memoryEntriesEmpty}
        </p>
      ) : (
        <div className="grid">
          {entries.map(entry => (
            <div data-settings-slot="" key={entry.id}>
              <ListRow
                action={
                  <Tip label={m.memoryRemove}>
                    <Button
                      aria-label={m.memoryRemove}
                      onClick={() => void remove(entry)}
                      size="icon-xs"
                      variant="ghost"
                    >
                      <Trash2 />
                    </Button>
                  </Tip>
                }
                description={[entry.category, entry.source].filter(Boolean).join(' · ')}
                inline
                title={<span className="line-clamp-3 whitespace-pre-wrap font-normal">{entry.content}</span>}
              />
            </div>
          ))}
        </div>
      )}
    </section>
  )
}
