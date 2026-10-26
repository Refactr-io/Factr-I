import { isDesktopFsRemoteMode } from '@/lib/desktop-fs'
import { normalizeOrLocalPreviewTarget } from '@/lib/local-preview'
import { $previewTabs, openPreview, renderedHtmlTarget } from '@/store/preview'

/**
 * An HTML file the agent wrote opens in the in-app browser pane, like any page it shows, instead of waiting for a
 * click. It opens by itself once per conversation and file: closing the tab is a choice that later edits of the same
 * file do not undo. An open tab needs nothing here, because a file tab already reloads when its file changes on disk.
 */

const HTML_EXTENSION = /\.html?$/i
const autoOpened = new Set<string>()

export type HtmlAutoOpen = 'already-open' | 'opened' | 'skipped' | 'unsupported'

/** A path or `file:` URL naming an HTML file (a web address is not one, whatever it ends in). */
export function isLocalHtmlTarget(target: string): boolean {
  const clean = target.trim().split(/[?#]/, 1)[0] ?? ''

  return HTML_EXTENSION.test(clean) && !/^(?:https?|data|blob):/i.test(clean)
}

export function resetHtmlAutoOpen(sessionKey?: string) {
  if (!sessionKey) {
    autoOpened.clear()

    return
  }

  for (const key of [...autoOpened]) {
    if (key.startsWith(`${sessionKey}\n`)) {
      autoOpened.delete(key)
    }
  }
}

export async function autoOpenHtmlFile(sessionKey: string, rawTarget: string, cwd?: string): Promise<HtmlAutoOpen> {
  // A file on the backend host is not one this machine can load into the pane.
  if (!isLocalHtmlTarget(rawTarget) || isDesktopFsRemoteMode()) {
    return 'unsupported'
  }

  const target = await normalizeOrLocalPreviewTarget(rawTarget, cwd)

  if (!target || target.kind !== 'file' || target.previewKind !== 'html') {
    return 'unsupported'
  }

  const identity = target.path ?? target.source

  if ($previewTabs.get().some(tab => tab.target.kind === 'file' && (tab.target.path ?? tab.target.source) === identity)) {
    return 'already-open'
  }

  const key = `${sessionKey}\n${identity}`

  if (autoOpened.has(key)) {
    return 'skipped'
  }

  autoOpened.add(key)
  openPreview(renderedHtmlTarget(target))

  return 'opened'
}
