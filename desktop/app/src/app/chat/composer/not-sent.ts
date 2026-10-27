import type { ConnectionState } from '@factr/shared'

import { translateNow } from '@/i18n'
import { notify } from '@/store/notifications'
import { $gatewayState } from '@/store/session'

const NOTICE_ID = 'composer-not-sent'

/** Which explanation fits a send that the engine connection cannot take. An open connection means the composer is off
 *  for another reason (a read-only run), which already says why in its placeholder. */
export function notSentReason(state: ConnectionState): 'starting' | 'stopped' | null {
  if (state === 'open') {
    return null
  }

  return state === 'closed' || state === 'error' ? 'stopped' : 'starting'
}

/** The composer keeps the typed text when it cannot send. Say so, instead of dropping Enter without a word. */
export function announceNotSent(state: ConnectionState = $gatewayState.get()) {
  const reason = notSentReason(state)

  if (!reason) {
    return
  }

  notify({
    id: NOTICE_ID,
    kind: 'warning',
    message: translateNow(reason === 'stopped' ? 'composer.notSentStopped' : 'composer.notSentStarting')
  })
}
