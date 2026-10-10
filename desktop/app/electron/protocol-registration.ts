/**
 * Whether this process may claim the OS `factr://` handler. A QA copy of the
 * app runs with FACTR_DISABLE_PROTOCOL_REGISTRATION=1 so it never takes the
 * deep-link handler away from the installed app.
 */
export const DISABLE_PROTOCOL_REGISTRATION_ENV = 'FACTR_DISABLE_PROTOCOL_REGISTRATION'

export function shouldRegisterProtocolClient(env: NodeJS.ProcessEnv): boolean {
  return env[DISABLE_PROTOCOL_REGISTRATION_ENV] !== '1'
}
