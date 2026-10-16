import { useStore } from '@nanostores/react'

import { Tip } from '@/components/ui/tooltip'
import { useI18n } from '@/i18n'
import { CircleArrowDownFilled, CircleArrowUp, Loader } from '@/lib/icons'
import { cn } from '@/lib/utils'
import { $autoDownload, installDownloadedUpdate, startDownloadNow } from '@/store/update-download'
import { $turnActive, $updateIndicator } from '@/store/update-indicator'
import { openUpdatesWindow } from '@/store/updates'

const RAIL_BUTTON =
  'grid size-8 place-items-center rounded-md text-(--ui-text-secondary) transition-colors duration-100 ease-out [-webkit-app-region:no-drag] active:scale-[0.97] hover:bg-(--ui-control-hover-background) hover:text-foreground'

/** A quiet, persistent mark that a newer release exists: one accent dot on a rail button, no glow, no pulse.
 *  It stays for as long as the update does (the toast snooze does not touch it) and opens the updates overlay. */
export function UpdateIndicator() {
  const { t } = useI18n()
  const { downloadable, kind, phase, version, visible } = useStore($updateIndicator)
  const autoDownload = useStore($autoDownload)
  const turnActive = useStore($turnActive)

  if (!visible) {
    return null
  }

  const n = t.notifications
  // A ready macOS update restarts the app in place, which must never cut a running task short.
  const restart = phase === 'ready' && kind === 'zip'
  const blocked = restart && turnActive

  const label = blocked
    ? n.updateBusyTip
    : restart
      ? n.updateRestartTip
      : phase === 'ready'
        ? n.updateInstallTip
        : phase === 'downloading' && version
          ? n.updateDownloadingTip(version)
          : n.updateAvailableTip(version)

  return (
    <Tip label={label} placement="left-rail">
      <button
        aria-label={label}
        className={cn(RAIL_BUTTON, 'relative', blocked && 'opacity-50')}
        data-phase={phase}
        data-tour="rail-update-available"
        disabled={blocked}
        onClick={() => {
          if (phase === 'ready') {
            void installDownloadedUpdate()
          } else if (phase === 'available' && !autoDownload && downloadable) {
            // Auto-update is off: the click is the user's go-ahead to download now.
            void startDownloadNow()
          } else {
            openUpdatesWindow('client')
          }
        }}
        type="button"
      >
        {phase === 'downloading' ? (
          // A slow, steady turn (static under reduced motion): quiet progress, no glow.
          <Loader className="size-4 motion-safe:animate-[spin_1.6s_linear_infinite]" />
        ) : phase === 'ready' ? (
          <CircleArrowDownFilled className="size-4 text-foreground" />
        ) : (
          <CircleArrowUp className="size-4" />
        )}
        {phase !== 'downloading' && (
          <span aria-hidden="true" className="absolute right-1 top-1 size-1.5 rounded-full bg-(--ui-accent)" />
        )}
      </button>
    </Tip>
  )
}
