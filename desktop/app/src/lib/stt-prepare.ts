import { getSttStatus, prepareStt, type SttPrepareStatus } from '@/api/system'
import { translateNow } from '@/i18n'
import { dismissNotification, notify } from '@/store/notifications'

/**
 * Dictation's speech engine and model are fetched when it is first used, never at startup. The first press of the
 * microphone starts that, and a notice follows it: getting ready, ready, or why it failed (with a retry).
 */

const NOTICE_ID = 'stt-prepare'
const POLL_MS = 1500

let running: Promise<SttPrepareStatus> | null = null

const settled = (state: SttPrepareStatus['state']) => state === 'ready' || state === 'not_needed' || state === 'failed'

function show(status: SttPrepareStatus) {
  if (status.state === 'installing' || status.state === 'downloading') {
    notify({
      detail: status.detail || undefined,
      durationMs: 0,
      id: NOTICE_ID,
      kind: 'info',
      message: translateNow('composer.dictationPreparing')
    })
  } else if (status.state === 'ready') {
    notify({ durationMs: 3000, id: NOTICE_ID, kind: 'success', message: translateNow('composer.dictationReady') })
  } else if (status.state === 'failed') {
    notify({
      action: { label: translateNow('composer.dictationRetry'), onClick: () => void ensureSttReady() },
      detail: status.detail || undefined,
      durationMs: 0,
      id: NOTICE_ID,
      kind: 'error',
      message: translateNow('composer.dictationFailed')
    })
  } else {
    dismissNotification(NOTICE_ID)
  }
}

/** Make sure local dictation is being prepared. Safe to call on every microphone press: it does nothing once the
 *  engine is ready (or not needed), and shares one preparation while it runs. Never throws. */
export function ensureSttReady(): Promise<SttPrepareStatus> {
  if (running) {
    return running
  }

  running = (async () => {
    try {
      let status = await getSttStatus()

      if (status.state === 'idle' || status.state === 'failed') {
        status = await prepareStt()
      }

      show(status)

      while (!settled(status.state)) {
        await new Promise(resolve => window.setTimeout(resolve, POLL_MS))
        status = await getSttStatus()
        show(status)
      }

      return status
    } catch {
      // An older backend without these routes: dictation works as before (it prepares itself on first use).
      return { detail: '', state: 'not_needed' as const }
    } finally {
      running = null
    }
  })()

  return running
}
