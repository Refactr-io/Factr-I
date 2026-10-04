import { useStore } from '@nanostores/react'
import { useMemo } from 'react'

import { Codicon } from '@/components/ui/codicon'
import { useContributions } from '@/contrib/react/use-contributions'
import { $interfaceMode, shownInMode } from '@/store/interface-mode'

import {
  ARTIFACTS_ROUTE,
  CAPABILITIES_ROUTE,
  CRON_ROUTE,
  MESSAGING_ROUTE,
  SIDEBAR_NAV_AREA,
  type SidebarNavContribution
} from '../../routes'
import type { SidebarNavItem } from '../../types'

// A row's `tier` is the one mode it belongs to (Simple keeps the setup rows,
// Advanced adds the readouts); the list filters once, nothing is passed down.
export const SIDEBAR_NAV: SidebarNavItem[] = [
  {
    id: 'new-session',
    label: '',
    icon: props => <Codicon name="new-session" {...props} />,
    action: 'new-session',
    keybindActionId: 'session.new'
  },
  {
    id: 'capabilities',
    label: '',
    icon: props => <Codicon name="symbol-misc" {...props} />,
    route: CAPABILITIES_ROUTE,
    keybindActionId: 'nav.capabilities'
  },
  {
    id: 'messaging',
    label: '',
    icon: props => <Codicon name="comment" {...props} />,
    route: MESSAGING_ROUTE,
    keybindActionId: 'nav.messaging'
  },
  // Artifacts and Scheduled jobs are outputs of running the agent the developer
  // way; Capabilities and Messaging are how anyone sets it up.
  {
    id: 'artifacts',
    label: '',
    icon: props => <Codicon name="files" {...props} />,
    route: ARTIFACTS_ROUTE,
    keybindActionId: 'nav.artifacts',
    tier: 'advanced'
  },
  {
    id: 'cron',
    label: '',
    icon: props => <Codicon name="watch" {...props} />,
    route: CRON_ROUTE,
    keybindActionId: 'nav.cron',
    tier: 'advanced'
  }
]

/** Built-in nav plus plugin-contributed rows, filtered to the interface mode.
 *  Contributed rows (plugins pairing a page with a sidebar entry) are active at
 *  their own route. */
export function useSidebarNavItems(): SidebarNavItem[] {
  const contributions = useContributions(SIDEBAR_NAV_AREA)
  const interfaceMode = useStore($interfaceMode)

  const contributed = useMemo<SidebarNavItem[]>(
    () =>
      contributions.flatMap(c => {
        const data = c.data as Partial<SidebarNavContribution> | undefined

        if (!data?.path?.startsWith('/') || !data.label) {
          return []
        }

        const codicon = data.codicon || 'plug'

        return [
          {
            id: c.id,
            label: data.label,
            icon: (props: { className?: string }) => <Codicon name={codicon} {...props} />,
            route: data.path,
            tier: data.tier
          }
        ]
      }),
    [contributions]
  )

  return useMemo(
    () => [...SIDEBAR_NAV, ...contributed].filter(shownInMode(interfaceMode)),
    [contributed, interfaceMode]
  )
}
