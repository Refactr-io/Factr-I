import type { SkillInfo, StarmapGraph } from '@/types/factr'

import { capabilityScoped, factrApi, type ProfileScope, profileScoped } from './client'

export function getSkills(profile?: ProfileScope): Promise<SkillInfo[]> {
  return window.factrDesktop.api<SkillInfo[]>({
    ...capabilityScoped(profile),
    path: '/api/skills'
  })
}

/** Raw SKILL.md text (frontmatter included) for ANY skill — bundled or
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
