import type {
  MessagingPlatformsResponse,
  MessagingPlatformTestResponse,
  MessagingPlatformUpdate,
  PairingResponse,
  PairingUser,
  WebhookCreatePayload,
  WebhookCreateResponse,
  WebhookEnableResponse,
  WebhooksResponse
} from '@/types/factr'

import { factrApi, profileScoped } from './client'

export function getMessagingPlatforms(profile?: null | string): Promise<MessagingPlatformsResponse> {
  return factrApi<MessagingPlatformsResponse>({
    ...profileScoped(profile),
    path: '/api/messaging/platforms'
  })
}

/** `hot_served`: a live multiplexer serving this named profile rebuilt its adapters from the new
 *  credentials right away — no gateway restart is needed for the change to take effect. */
export interface MessagingPlatformUpdateResponse {
  hot_served?: boolean
  ok: boolean
  platform: string
}

export function updateMessagingPlatform(
  platformId: string,
  body: MessagingPlatformUpdate,
  profile?: null | string
): Promise<MessagingPlatformUpdateResponse> {
  return factrApi<MessagingPlatformUpdateResponse>({
    ...profileScoped(profile),
    path: `/api/messaging/platforms/${encodeURIComponent(platformId)}`,
    method: 'PUT',
    body
  })
}

export function testMessagingPlatform(
  platformId: string,
  profile?: null | string
): Promise<MessagingPlatformTestResponse> {
  return factrApi<MessagingPlatformTestResponse>({
    ...profileScoped(profile),
    path: `/api/messaging/platforms/${encodeURIComponent(platformId)}/test`,
    method: 'POST'
  })
}

// -- Pairing (who may DM the bot) --------------------------------------------
// Unknown DMers get a one-time code and land in `pending` until an admin
// approves them. Approval grants on the row's `request_id`, never on the code:
// the code is the requester's proof that the channel is theirs and is never
// returned by the API, while an authenticated admin is only ever identifying
// a row they can already see.

export function getPairing(profile?: null | string): Promise<PairingResponse> {
  return factrApi<PairingResponse>({
    ...profileScoped(profile),
    path: '/api/pairing'
  })
}

export function approvePairing(
  platform: string,
  requestId: string,
  profile?: null | string
): Promise<{ ok: boolean; user: PairingUser }> {
  const scope = profileScoped(profile)

  return factrApi<{ ok: boolean; user: PairingUser }>({
    ...scope,
    path: '/api/pairing/approve',
    method: 'POST',
    // These endpoints read the profile off the body, not the query string —
    // the request scope alone would approve into the wrong profile's store.
    body: { platform, request_id: requestId, profile: scope.profile }
  })
}

export function revokePairing(platform: string, userId: string, profile?: null | string): Promise<{ ok: boolean }> {
  const scope = profileScoped(profile)

  return factrApi<{ ok: boolean }>({
    ...scope,
    path: '/api/pairing/revoke',
    method: 'POST',
    body: { platform, user_id: userId, profile: scope.profile }
  })
}

// -- Webhooks (subscription CRUD) --------------------------------------------
// The webhook receiver is its own gateway platform; subscriptions live in a
// shared JSON store the CLI/dashboard also drive. Enable mutates config and
// best-effort restarts the gateway; subscription changes hot-reload.

export function getWebhooks(): Promise<WebhooksResponse> {
  return factrApi<WebhooksResponse>({
    ...profileScoped(),
    path: '/api/webhooks'
  })
}

export function enableWebhooks(): Promise<WebhookEnableResponse> {
  return factrApi<WebhookEnableResponse>({
    ...profileScoped(),
    path: '/api/webhooks/enable',
    method: 'POST'
  })
}

export function createWebhook(body: WebhookCreatePayload): Promise<WebhookCreateResponse> {
  return factrApi<WebhookCreateResponse>({
    ...profileScoped(),
    path: '/api/webhooks',
    method: 'POST',
    body
  })
}

export function deleteWebhook(name: string): Promise<{ ok: boolean }> {
  return factrApi<{ ok: boolean }>({
    ...profileScoped(),
    path: `/api/webhooks/${encodeURIComponent(name)}`,
    method: 'DELETE'
  })
}

export function setWebhookEnabled(
  name: string,
  enabled: boolean
): Promise<{ enabled: boolean; name: string; ok: boolean }> {
  return factrApi<{ enabled: boolean; name: string; ok: boolean }>({
    ...profileScoped(),
    path: `/api/webhooks/${encodeURIComponent(name)}/enabled`,
    method: 'PUT',
    body: { enabled }
  })
}
