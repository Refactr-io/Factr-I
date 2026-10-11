import { atom } from 'nanostores'

/** Modes the engine accepts (factr-gateway rpc/settings.rs rejects `smart`; its default is `manual`). */
export type ApprovalMode = 'manual' | 'off'
export const DEFAULT_APPROVAL_MODE: ApprovalMode = 'manual'
export type ApprovalModeRequester = (method: string, params?: Record<string, unknown>) => Promise<unknown>

const APPROVAL_MODES = new Set<ApprovalMode>(['manual', 'off'])
const revisions = new Map<string, number>()
const confirmedModes = new Map<string, ApprovalMode>()

export const $approvalModes = atom<Record<string, ApprovalMode>>({})

function profileKey(profile: string): string {
  return profile.trim() || 'default'
}

function nextRevision(profile: string): number {
  const revision = (revisions.get(profile) ?? 0) + 1
  revisions.set(profile, revision)

  return revision
}

function normalizeApprovalMode(value: unknown): ApprovalMode {
  const normalized = String(value ?? '')
    .trim()
    .toLowerCase() as ApprovalMode

  // A stale `smart` in config.yaml is not honoured by the engine: it behaves as manual.
  return APPROVAL_MODES.has(normalized) ? normalized : DEFAULT_APPROVAL_MODE
}

/** True when the backend actually reported a mode. An unset key (not written to config.yaml yet)
 *  comes back empty and means "engine default", which is manual: keep what is shown. */
function hasReportedMode(value: unknown): boolean {
  return typeof value === 'string' && value.trim() !== ''
}

export function approvalModeForProfile(profile: string): ApprovalMode {
  return $approvalModes.get()[profileKey(profile)] ?? DEFAULT_APPROVAL_MODE
}

function cacheApprovalMode(profile: string, mode: ApprovalMode): void {
  const key = profileKey(profile)
  $approvalModes.set({ ...$approvalModes.get(), [key]: mode })
}

export function reconcileApprovalModeForProfile(profile: string, value: unknown): ApprovalMode {
  const key = profileKey(profile)

  if (!hasReportedMode(value)) {
    return approvalModeForProfile(key)
  }

  const mode = normalizeApprovalMode(value)
  nextRevision(key)
  confirmedModes.set(key, mode)
  cacheApprovalMode(key, mode)

  return mode
}

export async function syncApprovalModeForProfile(
  requestGateway: ApprovalModeRequester,
  profile: string
): Promise<ApprovalMode> {
  const key = profileKey(profile)
  const revision = nextRevision(key)
  const result = (await requestGateway('config.get', { key: 'approvals.mode' })) as { value?: string }
  const mode = hasReportedMode(result?.value) ? normalizeApprovalMode(result?.value) : approvalModeForProfile(key)

  if (hasReportedMode(result?.value) && revisions.get(key) === revision) {
    confirmedModes.set(key, mode)
    cacheApprovalMode(key, mode)
  }

  return mode
}

export async function setApprovalModeForProfile(
  requestGateway: ApprovalModeRequester,
  profile: string,
  mode: ApprovalMode
): Promise<ApprovalMode> {
  const key = profileKey(profile)
  const revision = nextRevision(key)
  cacheApprovalMode(key, mode)

  try {
    const result = (await requestGateway('config.set', {
      key: 'approvals.mode',
      value: mode
    })) as { value?: string }

    // An empty reply is not a report: keep what the user just picked instead of reading it as 'manual'.
    const authoritative = hasReportedMode(result?.value) ? normalizeApprovalMode(result?.value) : mode

    if (revisions.get(key) === revision) {
      confirmedModes.set(key, authoritative)
      cacheApprovalMode(key, authoritative)
    }

    return authoritative
  } catch (error) {
    if (revisions.get(key) === revision) {
      cacheApprovalMode(key, confirmedModes.get(key) ?? DEFAULT_APPROVAL_MODE)
    }

    throw error
  }
}
