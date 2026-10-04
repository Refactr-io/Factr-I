import type {
  OfficialSkillInfo,
  SkillHubPreview,
  SkillHubScanResult,
  SkillHubSearchResponse,
  SkillHubSourcesResponse,
  SkillInfo,
  StarmapGraph
} from '@/types/factr'
import type { ActionResponse } from '@/types/factr'

import { capabilityScoped, factrApi, type ProfileScope, profileScoped } from './client'

export function getSkills(profile?: ProfileScope): Promise<SkillInfo[]> {
  return window.factrDesktop.api<SkillInfo[]>({
    ...capabilityScoped(profile),
    path: '/api/skills'
  })
}

/** Raw SKILL.md text (frontmatter included) for ANY skill — bundled, hub, or
 *  learned — backing the Capabilities detail pane's full-skill view. */
export function getSkillContent(
  name: string,
  profile?: ProfileScope
): Promise<{ content: string; name: string; path: string }> {
  return window.factrDesktop.api<{ content: string; name: string; path: string }>({
    ...capabilityScoped(profile),
    path: `/api/skills/content?name=${encodeURIComponent(name)}`
  })
}

export function setSkillEnabled(
  name: string,
  enabled: boolean,
  profile?: ProfileScope
): Promise<{ ok: boolean; name: string; enabled: boolean }> {
  return window.factrDesktop.api<{ ok: boolean; name: string; enabled: boolean }>({
    ...capabilityScoped(profile),
    path: '/api/skills/toggle',
    method: 'PUT',
    body: { name, enabled }
  })
}

export function getStarmapGraph(): Promise<StarmapGraph> {
  return factrApi<StarmapGraph>({
    ...profileScoped(),
    // Backend REST contract — stays /api/learning even though the UI feature is
    // now "star map". Renaming this would break against an un-upgraded backend.
    path: '/api/learning/graph'
  })
}

export interface HarnessEntryRow {
  content: string
  id: string
  path: string
  scope: string
  source: string
  timestamp: number
  title: string
}

export interface HarnessChangeset {
  edits: number
  id: string
  isRollback: boolean
  rationale: string
  rolledBack: boolean
  summary: string
  timestamp: number
}

/** factr-learn's learned prompt entries and recent changesets (engine REST). */
export function getHarness(): Promise<{ changesets: HarnessChangeset[]; entries: HarnessEntryRow[] }> {
  return factrApi<{ changesets: HarnessChangeset[]; entries: HarnessEntryRow[] }>({
    ...profileScoped(),
    path: '/api/learning/harness'
  })
}

export function rollbackHarness(id: string): Promise<{ message: string; ok: boolean }> {
  return factrApi<{ message: string; ok: boolean }>({
    ...profileScoped(),
    path: '/api/learning/harness/rollback',
    method: 'POST',
    body: { id }
  })
}

export interface LearningNodeDetail {
  content: string
  kind: 'memory' | 'skill'
  label: string
  ok: boolean
}

export function getLearningNode(id: string, profile?: ProfileScope): Promise<LearningNodeDetail> {
  return window.factrDesktop.api<LearningNodeDetail>({
    ...capabilityScoped(profile),
    path: `/api/learning/node?id=${encodeURIComponent(id)}`
  })
}

export function deleteLearningNode(id: string, profile?: ProfileScope): Promise<{ message: string; ok: boolean }> {
  return window.factrDesktop.api<{ message: string; ok: boolean }>({
    ...capabilityScoped(profile),
    path: '/api/learning/node',
    method: 'DELETE',
    body: { id }
  })
}

export function editLearningNode(
  id: string,
  content: string,
  profile?: ProfileScope
): Promise<{ message: string; ok: boolean }> {
  return window.factrDesktop.api<{ message: string; ok: boolean }>({
    ...capabilityScoped(profile),
    path: '/api/learning/node',
    method: 'PUT',
    body: { content, id }
  })
}

// ---------------------------------------------------------------------------
// Skills hub — search / preview / scan / install (parity with `factr skills`
// and the dashboard's Browse-hub tab). Installs spawn background actions whose
// logs are tailed via getActionStatus().
// ---------------------------------------------------------------------------

const HUB_REQUEST_TIMEOUT_MS = 45_000

/** The full built-in optional-skills catalog (local checkout scan — fast),
 *  with per-profile installed flags. Feeds the Capabilities Skills list's
 *  "available to install" rows. */
export function getOfficialSkills(profile?: ProfileScope): Promise<{ skills: OfficialSkillInfo[] }> {
  return window.factrDesktop.api<{ skills: OfficialSkillInfo[] }>({
    ...capabilityScoped(profile),
    path: '/api/skills/hub/official'
  })
}

export function getSkillHubSources(profile?: null | string): Promise<SkillHubSourcesResponse> {
  return factrApi<SkillHubSourcesResponse>({
    ...profileScoped(profile),
    path: '/api/skills/hub/sources',
    timeoutMs: HUB_REQUEST_TIMEOUT_MS
  })
}

export function searchSkillsHub(
  query: string,
  source = 'all',
  limit = 20,
  profile?: null | string
): Promise<SkillHubSearchResponse> {
  const params = new URLSearchParams({ q: query, source, limit: String(limit) })

  return factrApi<SkillHubSearchResponse>({
    ...profileScoped(profile),
    path: `/api/skills/hub/search?${params.toString()}`,
    timeoutMs: HUB_REQUEST_TIMEOUT_MS
  })
}

export function previewSkillHub(identifier: string, profile?: ProfileScope): Promise<SkillHubPreview> {
  return window.factrDesktop.api<SkillHubPreview>({
    ...capabilityScoped(profile),
    path: `/api/skills/hub/preview?identifier=${encodeURIComponent(identifier)}`,
    timeoutMs: HUB_REQUEST_TIMEOUT_MS
  })
}

export function scanSkillHub(identifier: string, profile?: null | string): Promise<SkillHubScanResult> {
  return factrApi<SkillHubScanResult>({
    ...profileScoped(profile),
    path: `/api/skills/hub/scan?identifier=${encodeURIComponent(identifier)}`,
    timeoutMs: HUB_REQUEST_TIMEOUT_MS
  })
}

export function installSkillFromHub(identifier: string, profile?: ProfileScope): Promise<ActionResponse> {
  return window.factrDesktop.api<ActionResponse>({
    ...capabilityScoped(profile),
    path: '/api/skills/hub/install',
    method: 'POST',
    body: { identifier }
  })
}

export function uninstallSkillFromHub(name: string, profile?: ProfileScope): Promise<ActionResponse> {
  return window.factrDesktop.api<ActionResponse>({
    ...capabilityScoped(profile),
    path: '/api/skills/hub/uninstall',
    method: 'POST',
    body: { name }
  })
}

export function updateSkillsFromHub(profile?: ProfileScope): Promise<ActionResponse> {
  return window.factrDesktop.api<ActionResponse>({
    ...capabilityScoped(profile),
    path: '/api/skills/hub/update',
    method: 'POST',
    body: {}
  })
}
