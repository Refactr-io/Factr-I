import type { ChildProcess } from 'node:child_process'

// The bundled engine's session token must not sit in its environment: `ps eww <pid>` prints a process's
// initial environment to anything running as the same user, including the model's shell commands.
// So the engine gets it on stdin (`--token-stdin`, one line, then EOF). Python backends keep the env var.
export function engineTokenLaunch(kind: string | undefined, token: string) {
  if (kind !== 'factr') {
    return {
      args: [] as string[],
      env: { FACTR_DASHBOARD_SESSION_TOKEN: token } as Record<string, string | undefined>,
      stdio: ['ignore', 'pipe', 'pipe'] as ['ignore', 'pipe', 'pipe'] | ['pipe', 'pipe', 'pipe'],
      deliver: (_child: ChildProcess) => {}
    }
  }

  return {
    args: ['--token-stdin'],
    env: { FACTR_DASHBOARD_SESSION_TOKEN: undefined } as Record<string, string | undefined>,
    stdio: ['pipe', 'pipe', 'pipe'] as ['ignore', 'pipe', 'pipe'] | ['pipe', 'pipe', 'pipe'],
    deliver: (child: ChildProcess) => {
      child.stdin?.on('error', () => {})
      child.stdin?.end(`${token}\n`)
    }
  }
}
