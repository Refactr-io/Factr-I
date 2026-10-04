import path from 'node:path'

/** The engine's data dir: `FACTR_HOME`, else `~/.factr/engine` (as `factr_dir()` resolves it). */
export function factrHome(env: Record<string, string | undefined>, homedir: string): string {
  return env.FACTR_HOME || path.join(homedir, '.factr/engine')
}
