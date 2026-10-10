/**
 * The provider flags the desktop starts the bundled engine with.
 *
 * Only an explicit launch choice (FACTR_PROVIDER / FACTR_MODEL) becomes a flag. Without one the
 * engine decides: the pick saved in config.yaml, else the signed-in provider. A flag the desktop
 * invented would outrank the user's saved pick, so none is ever invented here.
 */
export function engineProviderArgs(env: Record<string, string | undefined>): string[] {
  const provider = env.FACTR_PROVIDER?.trim() || ''
  const model = env.FACTR_MODEL?.trim() || ''

  return [...(provider ? ['--provider', provider] : []), ...(model ? ['--model', model] : [])]
}
