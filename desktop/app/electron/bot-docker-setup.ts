/** Docker setup belongs to the Desktop. Every command is fixed argv, and runs only after consent. */
export type DockerSetupState = 'not-installed' | 'installed-not-running' | 'starting' | 'ready' | 'error'
export interface DockerSetupStatus { state: DockerSetupState; message?: string }
export interface SetupCommand { program: string; args: string[] }
export interface DockerInstallPlan {
  title: string
  detail: string
  commands: SetupCommand[]
  download?: string
  restart?: boolean
}
const DOCS = 'https://docs.docker.com/desktop/setup/install/'

export function dockerInstallPlan(platform: string, available: (name: string) => boolean, wslReady = true): DockerInstallPlan {
  if (platform === 'darwin') {
    return available('brew')
      ? { title: 'Install Docker Desktop', detail: 'Homebrew will install Docker Desktop. Docker may ask you to accept its terms.', commands: [{ program: 'brew', args: ['install', '--cask', 'docker'] }] }
      : { title: 'Download Docker Desktop', detail: 'Open Docker’s official installer. Complete installation, then return here.', commands: [], download: DOCS + 'mac-install/' }
  }
  if (platform === 'win32') {
    if (!wslReady) return {
      title: 'Set up WSL 2', detail: 'Windows may ask for administrator permission and a restart. No Linux distribution will be installed.', restart: true,
      commands: [{ program: 'powershell.exe', args: ['-NoProfile', '-Command', "Start-Process wsl.exe -ArgumentList '--install','--no-distribution' -Verb RunAs -Wait"] }]
    }
    return available('winget.exe')
      ? { title: 'Install Docker Desktop', detail: 'Windows Package Manager will open Docker’s installer. Review its terms and any Windows permission prompts.', commands: [{ program: 'winget.exe', args: ['install', '--exact', '--id', 'Docker.DockerDesktop', '--source', 'winget', '--interactive'] }] }
      : { title: 'Download Docker Desktop', detail: 'Open Docker’s official installer. Complete installation, then return here.', commands: [], download: DOCS + 'windows-install/' }
  }
  if (platform === 'linux' && available('apt-get') && available('pkexec')) return {
    title: 'Install Docker Engine', detail: 'Your system package manager will install docker.io and start its service. Linux will ask for administrator permission. Docker socket access may need a separate administrator step.',
    commands: [{ program: 'pkexec', args: ['apt-get', 'update'] }, { program: 'pkexec', args: ['apt-get', 'install', '-y', 'docker.io'] }]
  }
  return { title: 'Set up a container engine', detail: 'Open the installation instructions for your system. Return after Docker Engine or a Docker-compatible Podman service is ready.', commands: [], download: 'https://docs.docker.com/engine/install/' }
}

export function hasWsl2(version: string): boolean {
  const match = version.match(/(\d+)\.(\d+)\.(\d+)/)
  if (!match) return false
  const [major, minor, patch] = match.slice(1).map(Number)
  return major > 2 || (major === 2 && (minor > 1 || (minor === 1 && patch >= 5)))
}

export interface DockerSetupDeps {
  platform: string
  available: (program: string) => boolean
  run: (program: string, args: string[], timeoutMs: number) => Promise<string>
  probe: () => Promise<unknown>
  confirm: (plan: DockerInstallPlan) => Promise<boolean>
  openExternal: (url: string) => Promise<unknown>
  sleep?: (ms: number) => Promise<void>
  now?: () => number
}

export function createDockerSetup(deps: DockerSetupDeps) {
  let status: DockerSetupStatus = { state: 'not-installed' }
  let operation: Promise<DockerSetupStatus> | undefined
  const now = deps.now || Date.now
  const sleep = deps.sleep || ((ms: number) => new Promise<void>(resolve => setTimeout(resolve, ms)))
  const probe = async (): Promise<DockerSetupStatus> => {
    try { await deps.probe(); return { state: 'ready' } }
    catch (error: any) {
      return error?.code === 'ENOENT' ? { state: 'not-installed' }
        : /permission denied|access denied/i.test(String(error?.stderr || error?.message))
          ? { state: 'error', message: 'Docker is installed, but access is denied. Ask your administrator to allow access, or use a local sandbox.' }
          : { state: 'installed-not-running' }
    }
  }
  const start = (): SetupCommand => deps.platform === 'darwin'
    ? deps.available('Docker.app') || !deps.available('colima') ? { program: '/usr/bin/open', args: ['-a', 'Docker'] } : { program: 'colima', args: ['start'] }
    : deps.platform === 'win32' ? { program: 'docker', args: ['desktop', 'start'] }
      : { program: 'pkexec', args: ['systemctl', 'start', 'docker'] }

  async function setup(): Promise<DockerSetupStatus> {
    status = await probe()
    if (status.state === 'ready') return status
    let wslReady = true
    if (deps.platform === 'win32') {
      try { wslReady = hasWsl2(await deps.run('wsl.exe', ['--version'], 10000)) } catch { wslReady = false }
    }
    const plan = status.state === 'not-installed' || !wslReady
      ? dockerInstallPlan(deps.platform, deps.available, wslReady)
      : { title: 'Start Docker', detail: 'Start your installed container engine so this bot can use its own computer.', commands: [] }
    if (!await deps.confirm(plan)) return status
    if (plan.download) {
      await deps.openExternal(plan.download)
      return status = { state: 'not-installed', message: 'Complete the official installation, then try again.' }
    }
    status = { state: 'starting', message: plan.commands.length ? 'Installing the container engine…' : 'Starting the container engine…' }
    try {
      for (const command of plan.commands) await deps.run(command.program, command.args, 20 * 60_000)
      if (plan.restart) return status = { state: 'error', message: 'Finish WSL 2 setup and restart Windows if requested, then try again.' }
      if ((await probe()).state !== 'ready') {
        const command = start()
        await deps.run(command.program, command.args, 120000)
      }
      const deadline = now() + 120000
      do {
        const current = await probe()
        if (current.state === 'ready' || current.state === 'error') return status = current
        await sleep(1000)
      } while (now() < deadline)
      return status = { state: 'error', message: 'The container engine did not become ready. Finish any installer prompts, then try again.' }
    } catch (error: any) {
      return status = { state: 'error', message: String(error?.stderr || error?.message || error).trim().slice(-600) || 'Setup did not complete. Try again.' }
    }
  }
  return {
    async status() {
      if (operation) return status
      const current = await probe()
      if (current.state === 'ready' || status.state !== 'error') status = current
      return status
    },
    setup() {
      if (!operation) operation = setup().catch((error: any) => status = { state: 'error', message: String(error?.message || error) }).finally(() => { operation = undefined })
      return operation
    }
  }
}
