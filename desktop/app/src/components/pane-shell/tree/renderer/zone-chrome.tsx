import type { ReactNode } from 'react'

/**
 * A pane may contribute the CHROME of its zone (`data.zoneChrome`): a lead row
 * at the top of the zone's panel (the sessions list's product name + search).
 * The zone then has no tab strip - its panes are destinations on the app's icon
 * rail - and its body sits on the list surface (`--shell-list-bg`), one tone off
 * the main pane.
 */
export interface ZoneChrome {
  lead: () => ReactNode
}

export function ZoneChromeFrame({ children, chrome }: { children: ReactNode; chrome: null | ZoneChrome }) {
  if (!chrome) {
    return <>{children}</>
  }

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col bg-(--shell-list-bg)" data-zone-panel="">
      <div className="flex h-11 shrink-0 items-center">{chrome.lead()}</div>
      {children}
    </div>
  )
}
