import type { BillingBlock } from '@factr/shared'
import { atom } from 'nanostores'

import { openExternalLink } from '@/lib/external-link'
import { requestRoute } from '@/store/recovery-requests'

const PROVIDERS_SETTINGS_ROUTE = '/settings?tab=providers'

/**
 * The active inference billing wall, if any. Set from the gateway
 * `message.complete` / `error` event when a turn fails with
 * `FailoverReason.billing` (see `agent/billing_links.py`). One global slot: a
 * credit wall on the active session's provider is the whole app's problem, and
 * the newest block wins. Cleared when a new turn starts or the user dismisses.
 */
export interface ActiveBillingBlock {
  block: BillingBlock
  sessionId: string
  at: number
}

export const $billingBlock = atom<ActiveBillingBlock | null>(null)

export function setBillingBlock(sessionId: string, block: BillingBlock): void {
  $billingBlock.set({ at: Date.now(), block, sessionId })
}

export function clearBillingBlock(sessionId?: string): void {
  const current = $billingBlock.get()

  if (!current) {
    return
  }

  // A scoped clear (new turn on session X) must not wipe a block raised by a
  // different session's provider.
  if (sessionId && current.sessionId !== sessionId) {
    return
  }

  $billingBlock.set(null)
}

/**
 * The single recovery action for a billing wall, shared by the toast and the
 * in-chat banner so both behave identically: the provider's own billing page
 * when the backend named one, otherwise Settings → Providers so the user can
 * switch to another provider.
 */
export function runBillingRecovery(block: BillingBlock): void {
  if (block.billing_url) {
    openExternalLink(block.billing_url)

    return
  }

  requestRoute(PROVIDERS_SETTINGS_ROUTE)
}

export function billingCtaLabel(block: BillingBlock, copy: { addCredits: string; openProviders: string }): string {
  return block.billing_url ? copy.addCredits : copy.openProviders
}
