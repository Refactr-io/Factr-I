import { Fragment, useEffect, useMemo, useState } from 'react'

import { Button } from '@/components/ui/button'
import { Codicon } from '@/components/ui/codicon'
import { Dialog, DialogContent, DialogDescription, DialogTitle } from '@/components/ui/dialog'
import { useI18n } from '@/i18n'
import { readDesktopDir, setDesktopFsRemotePicker } from '@/lib/desktop-fs'
import { displayPath, pathLeaf } from '@/lib/display-path'
import { cn } from '@/lib/utils'
import { readableError } from '@/store/notifications'

function clean(path: string) {
  return path.replace(/\/+$/, '') || '/'
}

function parentDir(path: string) {
  const value = clean(path)

  if (value === '/') {
    return '/'
  }

  const parent = value.slice(0, value.lastIndexOf('/'))

  return parent || '/'
}

function pathName(path: string) {
  return pathLeaf(path) || path
}

interface PendingSelection {
  defaultPath: string
  resolve: (paths: string[]) => void
  title: string
}

export function RemoteFolderPicker() {
  const { t } = useI18n()
  const r = t.rightSidebar
  const [pending, setPending] = useState<PendingSelection | null>(null)
  const [currentPath, setCurrentPath] = useState('/')
  const [entries, setEntries] = useState<Array<{ name: string; path: string }>>([])
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(false)

  useEffect(() => {
    setDesktopFsRemotePicker({
      selectPaths: options =>
        new Promise(resolve => {
          const defaultPath = clean(options?.defaultPath || '/')
          setCurrentPath(defaultPath)
          setPending({ defaultPath, resolve, title: options?.title || r.remotePickerTitle })
        })
    })

    return () => setDesktopFsRemotePicker(null)
  }, [r.remotePickerTitle])

  useEffect(() => {
    if (!pending) {
      return
    }

    let active = true
    setLoading(true)
    setError(null)

    void readDesktopDir(currentPath)
      .then(result => {
        if (!active) {
          return
        }

        if (result.error) {
          setError(result.error)
          setEntries([])

          return
        }

        setEntries(
          result.entries.filter(entry => entry.isDirectory).map(entry => ({ name: entry.name, path: entry.path }))
        )
      })
      .catch(err => {
        if (active) {
          setError(readableError(err, 'read-error').message)
          setEntries([])
        }
      })
      .finally(() => {
        if (active) {
          setLoading(false)
        }
      })

    return () => {
      active = false
    }
  }, [currentPath, pending])

  const crumbs = useMemo(() => {
    const parts = clean(currentPath).split('/').filter(Boolean)
    const out = [{ label: '/', path: '/' }]
    let acc = ''

    for (const part of parts) {
      acc += `/${part}`
      out.push({ label: part, path: acc })
    }

    return out
  }, [currentPath])

  const close = (paths: string[] = []) => {
    pending?.resolve(paths)
    setPending(null)
    setEntries([])
    setError(null)
  }

  return (
    <Dialog onOpenChange={open => !open && close()} open={Boolean(pending)}>
      <DialogContent
        bodyClassName="flex min-h-0 flex-col gap-0 overflow-hidden p-0"
        className="max-w-lg"
      >
        <div className="shrink-0 border-b border-border/70 px-4 py-3 pr-12">
          <DialogTitle className="text-[length:var(--conversation-text-font-size)] leading-5 font-medium">{pending?.title || r.remotePickerTitle}</DialogTitle>
          <DialogDescription className="mt-0.5 text-[0.8125rem] leading-[1.125rem]">{r.remotePickerDescription}</DialogDescription>
        </div>

        <div className="flex min-h-0 flex-1 flex-col">
          <nav
            aria-label={pending?.title || r.remotePickerTitle}
            className="flex shrink-0 flex-wrap items-center gap-x-0.5 gap-y-1 border-b border-border/50 px-3 py-2 text-[length:var(--conversation-text-font-size)]"
          >
            {crumbs.map((crumb, index) => {
              const last = index === crumbs.length - 1

              return (
                <Fragment key={crumb.path}>
                  {index > 0 && <Codicon className="text-(--ui-text-tertiary)" name="chevron-right" size="0.75rem" />}
                  <button
                    aria-current={last ? 'location' : undefined}
                    className={cn(
                      'max-w-40 truncate rounded-md px-1.5 py-0.5 leading-5 transition-colors hover:bg-(--chrome-action-hover) hover:text-foreground',
                      last ? 'font-medium text-foreground' : 'text-(--ui-text-secondary)'
                    )}
                    onClick={() => setCurrentPath(crumb.path)}
                    type="button"
                  >
                    {crumb.label}
                  </button>
                </Fragment>
              )
            })}
          </nav>

          <div className="max-h-[min(20rem,calc(100vh-18rem))] min-h-40 flex-1 overflow-y-auto p-2">
            <FolderRow
              disabled={currentPath === '/'}
              icon="arrow-up"
              name={r.remotePickerParent}
              onClick={() => setCurrentPath(parentDir(currentPath))}
            />
            {loading ? (
              <div className="flex items-center gap-2 px-2 py-3 text-xs text-muted-foreground">
                <Codicon name="loading" size="0.8rem" spinning />
                {r.loadingFiles}
              </div>
            ) : error ? (
              <div className="px-2 py-3 text-xs text-destructive">{r.unreadableBody(error)}</div>
            ) : entries.length === 0 ? (
              <div className="px-2 py-3 text-xs text-muted-foreground">{r.emptyBody}</div>
            ) : (
              entries.map(entry => (
                <FolderRow key={entry.path} name={pathName(entry.path)} onClick={() => setCurrentPath(entry.path)} />
              ))
            )}
          </div>
        </div>

        <div className="shrink-0 flex items-center justify-between gap-2 border-t border-border/70 px-4 py-3">
          <div className="min-w-0 truncate text-[0.8125rem] text-(--ui-text-secondary)">{displayPath(currentPath)}</div>
          <div className="flex shrink-0 items-center gap-2">
            <Button onClick={() => close()} size="sm" variant="ghost">
              {t.common.cancel}
            </Button>
            <Button onClick={() => close([currentPath])} size="sm">
              {r.remotePickerSelect}
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  )
}

function FolderRow({
  disabled = false,
  icon = 'folder',
  name,
  onClick
}: {
  disabled?: boolean
  icon?: string
  name: string
  onClick: () => void
}) {
  return (
    <button
      className="row-hover flex h-9 w-full items-center gap-2.5 rounded-md px-2 text-left text-[length:var(--conversation-text-font-size)] text-(--ui-text-secondary) hover:text-foreground disabled:pointer-events-none disabled:opacity-40"
      disabled={disabled}
      onClick={onClick}
      type="button"
    >
      <Codicon name={icon} size="1rem" />
      <span className="min-w-0 truncate">{name}</span>
    </button>
  )
}
