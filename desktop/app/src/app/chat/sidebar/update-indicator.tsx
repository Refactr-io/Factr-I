import { useStore } from '@nanostores/react'

import { Tip } from '@/components/ui/tooltip'
import { useI18n } from '@/i18n'
import { CircleArrowUp } from '@/lib/icons'
import { cn } from '@/lib/utils'
import { $updateIndicator } from '@/store/update-indicator'
import { openUpdatesWindow } from '@/store/updates'

const RAIL_BUTTON =
  'grid size-8 place-items-center rounded-md text-(--ui-text-secondary) transition-colors duration-100 ease-out [-webkit-app-region:no-drag] active:scale-[0.97] hover:bg-(--ui-control-hover-background) hover:text-foreground'

/** A quiet, persistent mark that a newer release exists: one accent dot on a rail button, no glow, no pulse.
 *  It stays for as long as the update does (the toast snooze does not touch it) and opens the updates overlay. */
export function UpdateIndicator() {
  const { t } = useI18n()
  const { visible, version } = useStore($updateIndicator)

  if (!visible) {
    return null
  }

  const label = t.notifications.updateAvailableTip(version)

  return (
    <Tip label={label} placement="left-rail">
      <button
        aria-label={label}
        className={cn(RAIL_BUTTON, 'relative')}
        data-tour="rail-update-available"
        onClick={() => openUpdatesWindow('client')}
        type="button"
      >
        <CircleArrowUp className="size-4" />
        <span aria-hidden="true" className="absolute right-1 top-1 size-1.5 rounded-full bg-(--ui-accent)" />
      </button>
    </Tip>
  )
}

