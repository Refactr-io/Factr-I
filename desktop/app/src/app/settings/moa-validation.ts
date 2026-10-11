export const MOA_PRESET_NAME_MAX = 40

const MOA_PRESET_NAME_RE = /^[\p{L}\p{N} _.-]+$/u

export type MoaPresetNameProblem = 'chars' | 'duplicate' | 'empty' | 'long' | null

/** Preset names become YAML keys: keep them short and plain (letters, digits, space, dot, dash, underscore). */
export function moaPresetNameProblem(raw: string, existing: Record<string, unknown>): MoaPresetNameProblem {
  const name = raw.trim()

  if (!name) {
    return 'empty'
  }

  if (name.length > MOA_PRESET_NAME_MAX) {
    return 'long'
  }

  if (!MOA_PRESET_NAME_RE.test(name)) {
    return 'chars'
  }

  return existing[name] ? 'duplicate' : null
}

/** A provider the catalog marks as having no credentials cannot serve a MoA slot. */
export function moaProviderLacksCredentials(provider: { authenticated?: boolean | null } | undefined): boolean {
  return provider?.authenticated === false
}
