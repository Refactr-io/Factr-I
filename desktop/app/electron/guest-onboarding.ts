// Guest onboarding is gated by ONE launch-time decision, made by the desktop (env or
// `--guest-onboarding` argv) and handed to the renderer. No backend reads it.

export const GUEST_ONBOARDING_ENV = 'FACTR_GUEST_ONBOARDING'
export const GUEST_ONBOARDING_FLAG = '--guest-onboarding'
// Skip the first-run film. A rehearsal aid: the intro is a one-time reveal,
// so anyone iterating on the guided chat behind it otherwise sits through it
// on every fresh FACTR_CONFIG_HOME. The guide still runs — only the film is
// skipped. Renderer-only; the backend never sees it.
export const SKIP_INTRO_ENV = 'FACTR_SKIP_INTRO'
export const SKIP_INTRO_FLAG = '--skip-intro'

export function guestOnboardingEnabled(
  argv: readonly string[] = process.argv,
  env: NodeJS.ProcessEnv = process.env
): boolean {
  return env[GUEST_ONBOARDING_ENV] === '1' || argv.includes(GUEST_ONBOARDING_FLAG)
}

export function skipIntroEnabled(
  argv: readonly string[] = process.argv,
  env: NodeJS.ProcessEnv = process.env
): boolean {
  return env[SKIP_INTRO_ENV] === '1' || argv.includes(SKIP_INTRO_FLAG)
}
