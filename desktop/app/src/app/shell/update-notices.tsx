import { useStore } from '@nanostores/react'

import { Button } from '@/components/ui/button'
import { useI18n } from '@/i18n'
import { X } from '@/lib/icons'
import { installDownloadedUpdate } from '@/store/update-download'
import { $updateBanner, $updateBannerDismissed, $updatedNote, dismissUpdatedNote } from '@/store/update-notices'

const ROW =
  'mb-1.5 flex shrink-0 items-center gap-3 rounded-lg border border-(--shell-block-edge) bg-(--shell-block-bg) px-3 py-2 text-[length:var(--conversation-text-font-size)] text-foreground [-webkit-app-region:no-drag]'

const CLOSE =
  'ml-auto grid size-6 shrink-0 place-items-center rounded-md text-(--ui-text-secondary) transition-colors duration-100 hover:bg-(--ui-control-hover-background) hover:text-foreground'

/**
 * Slim rows above the app body: "Restart to update" once per launch while a verified macOS update waits, and a
 * one-time "Updated to X" after the restart. Both are dismissible and neither ever shows an error.
 */
export function UpdateNotices() {
  const { t } = useI18n()
  const banner = useStore($updateBanner)
  const note = useStore($updatedNote)
  const n = t.notifications

  return (
    <>
      {banner.visible && banner.version && (
        <div className={ROW} data-slot="update-ready-banner" role="status">
          <span>{n.updateBannerReady(banner.version)}</span>
          <Button onClick={() => void installDownloadedUpdate()} size="sm">
            {n.updateBannerRestart}
          </Button>
          <button aria-label={t.common.close} className={CLOSE} onClick={() => $updateBannerDismissed.set(true)} type="button">
            <X className="size-3.5" />
          </button>
        </div>
      )}
      {note && (
        <div className={ROW} data-slot="updated-note" role="status">
          <span>{n.updatedTo(note.version)}</span>
          <button aria-label={t.common.close} className={CLOSE} onClick={dismissUpdatedNote} type="button">
            <X className="size-3.5" />
          </button>
        </div>
      )}
    </>
  )
}
