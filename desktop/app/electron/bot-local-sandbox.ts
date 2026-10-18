import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

import { validateBotProfile } from './bot-computers'

export interface BotSandboxLaunch {
  command: string
  args: string[]
  cwd: string
  env: NodeJS.ProcessEnv
  cleanup: () => void
}

const quote = (value: string) => JSON.stringify(value)

/** The whole engine is confined, so native file tools and all shell descendants share the boundary. */
export function seatbeltProfile(root: string, temp: string, reads: string[]): string {
  const paths = [...new Set([root, temp, ...reads])]
  return `(version 1)
(deny default)
(import "system.sb")
(allow process-exec process-fork)
(allow signal (target self) (target same-sandbox))
(allow process-info* (target self) (target same-sandbox))
(allow sysctl-read)
(allow system-info)
(allow mach-lookup (global-name "com.apple.windowserver.active") (global-name "com.apple.lsd.mapdb")
  (global-name "com.apple.coreservices.launchservicesd")
  (global-name "com.apple.CoreServices.coreservicesd")
  (global-name "com.apple.distributed_notifications@Uv3"))
(deny lsopen)
(allow iokit-open-user-client (iokit-user-client-class "RootDomainUserClient"))
(allow mach-register (global-name-prefix "org.chromium.crashpad.")
  (global-name-prefix "com.google.chrome.for.testing.MachPortRendezvousServer.")
  (global-name-prefix "com.google.Chrome.MachPortRendezvousServer.")
  (global-name-prefix "org.chromium.Chromium.MachPortRendezvousServer."))
(allow mach-lookup (global-name-prefix "org.chromium.crashpad.")
  (global-name-prefix "com.google.chrome.for.testing.MachPortRendezvousServer.")
  (global-name-prefix "com.google.Chrome.MachPortRendezvousServer.")
  (global-name-prefix "org.chromium.Chromium.MachPortRendezvousServer."))
(system-network)
(allow mach-lookup (global-name "com.apple.SystemConfiguration.configd"))
(allow network-inbound network-outbound (local ip "*:*"))
(allow network-outbound (remote ip "*:*"))
(allow network-bind (local ip "localhost:*"))
(allow network* (subpath ${quote(temp)}))
(allow file-read-metadata)
(allow file-read* file-map-executable
  ${[...paths, '/bin', '/sbin', '/usr', '/opt/homebrew', '/Library/Apple', '/private/etc'].map(p => `(subpath ${quote(p)})`).join('\n  ')})
(allow file-read* (subpath "/dev"))
(allow file-write* (subpath ${quote(root)}) (subpath ${quote(temp)})
  (literal "/dev/null") (literal "/dev/zero") (subpath "/dev/fd"))
(deny mach-lookup (global-name "com.apple.securityd") (global-name "com.apple.SecurityServer")
  (global-name "com.apple.secd") (global-name "com.apple.security.agentd") (global-name "com.apple.pasteboard.1"))
`
}

export function sandboxCommand(
  platform: string,
  command: string,
  args: string[],
  root: string,
  temp: string,
  reads: string[],
  exists: (file: string) => boolean = fs.existsSync,
  windows?: { python: string; script: string }
): { command: string; args: string[] } {
  if (platform === 'darwin' && exists('/usr/bin/sandbox-exec')) {
    return { command: '/usr/bin/sandbox-exec', args: ['-p', seatbeltProfile(root, temp, reads), command, ...args] }
  }
  if (platform === 'linux') {
    const bwrap = ['/usr/bin/bwrap', '/bin/bwrap'].find(exists)
    if (bwrap) {
      const system = ['/usr', '/bin', '/sbin', '/lib', '/lib64', '/etc', '/opt'].filter(exists)
      return {
        command: bwrap,
        args: [
          '--die-with-parent',
          '--new-session',
          '--unshare-user',
          '--unshare-pid',
          '--unshare-ipc',
          '--unshare-uts',
          '--cap-drop',
          'ALL',
          '--proc',
          '/proc',
          '--dev',
          '/dev',
          '--tmpfs',
          '/tmp',
          ...[...new Set([...system, ...reads])].flatMap(p => ['--ro-bind', p, p]),
          '--bind',
          root,
          root,
          '--bind',
          temp,
          temp,
          '--chdir',
          path.join(root, 'workspace'),
          '--',
          command,
          ...args
        ]
      }
    }
  }
  if (platform === 'win32' && windows && exists(windows.python) && exists(windows.script)) {
    return {
      command: windows.python,
      args: ['-I', windows.script, JSON.stringify({ command, args, root, temp, reads })]
    }
  }
  throw new Error(
    'local-sandbox-unavailable: no supported OS sandbox is available. Set up Docker to run this bot safely.'
  )
}

function linkDirectory(target: string, link: string) {
  if (!fs.existsSync(link)) fs.symlinkSync(target, link, process.platform === 'win32' ? 'junction' : 'dir')
  if (fs.realpathSync(link) !== fs.realpathSync(target))
    throw new Error('The bot sandbox contains an unexpected directory link.')
}

/** Only call for a local managed bot after container/remote routing has declined it. */
export function prepareBotSandbox(options: {
  profile: string
  configHome: string
  command: string
  args: string[]
  env: NodeJS.ProcessEnv
  readRoots: string[]
}): BotSandboxLaunch {
  const { profile, command, args } = options
  validateBotProfile(profile)
  const profiles = fs.realpathSync(path.join(options.configHome, 'profiles'))
  const root = fs.realpathSync(path.join(profiles, profile))
  if (root !== path.join(profiles, profile)) throw new Error('A sandboxed bot must have its own profile directory.')
  if (fs.existsSync(path.join(root, '.computer')))
    throw new Error('This profile belongs to a Docker computer. Start Docker to reconnect it.')
  const local = path.join(root, '.local-computer')
  const config = path.join(local, 'config')
  const cwd = path.join(root, 'workspace')
  for (const dir of [
    cwd,
    path.join(root, 'engine'),
    path.join(root, 'browser-profile'),
    local,
    path.join(local, 'home'),
    config,
    path.join(config, 'profiles'),
    path.join(root, 'bot_relay')
  ]) {
    fs.mkdirSync(dir, { recursive: true, mode: 0o700 })
    if (!fs.realpathSync(dir).startsWith(root + path.sep))
      throw new Error('A bot directory points outside its sandbox.')
  }
  // Give roster/relay code the same single-profile install layout as a container, with durable mail in the profile.
  linkDirectory(root, path.join(config, 'profiles', profile))
  linkDirectory(path.join(root, 'bot_relay'), path.join(config, 'bot_relay'))
  const temp = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'bots-local-')))
  const cleanup = () => fs.rmSync(temp, { recursive: true, force: true })
  try {
    const reads = [...new Set(options.readRoots.map(p => fs.realpathSync(p)))]
    const source = options.env.FACTR_BACKEND_PYTHONPATH?.split(path.delimiter)[0]
    const windows =
      source && options.env.FACTR_BACKEND_PYTHON
        ? { python: options.env.FACTR_BACKEND_PYTHON, script: path.join(source, 'tools', 'windows_bot_sandbox.py') }
        : undefined
    const launch = sandboxCommand(process.platform, command, args, root, temp, reads, fs.existsSync, windows)
    const env: NodeJS.ProcessEnv = {
      ...options.env,
      HOME: path.join(local, 'home'),
      USERPROFILE: path.join(local, 'home'),
      FACTR_REAL_HOME: path.join(local, 'home'),
      ...(process.platform === 'win32'
        ? {
            APPDATA: path.join(local, 'home', 'AppData', 'Roaming'),
            LOCALAPPDATA: path.join(local, 'home', 'AppData', 'Local')
          }
        : {}),
      CFFIXED_USER_HOME: path.join(local, 'home'),
      CFPREFERENCES_AVOID_DAEMON: '1',
      FACTR_HOME: path.join(root, 'engine'),
      FACTR_CONFIG_HOME: config,
      FACTR_LAZY_INSTALL_TARGET: path.join(local, 'lazy-packages'),
      FACTR_RUNTIME_DIR: temp,
      TMPDIR: temp,
      TMP: temp,
      TEMP: temp,
      MAC_CHROMIUM_TMPDIR: temp,
      XDG_CONFIG_HOME: path.join(local, 'home', '.config'),
      XDG_CACHE_HOME: path.join(local, 'home', '.cache'),
      XDG_DATA_HOME: path.join(local, 'home', '.local', 'share'),
      FACTR_DESKTOP_CWD: cwd,
      TERMINAL_CWD: cwd,
      AGENT_BROWSER_PROFILE: path.join(root, 'browser-profile'),
      PLAYWRIGHT_BROWSERS_PATH: path.join(local, 'browsers'),
      // Seatbelt cannot nest the REPL or Chromium sandbox. Chromium inherits this policy and uses bot-only key storage.
      ...(process.platform === 'darwin'
        ? {
            FACTR_REPL: '0',
            AGENT_BROWSER_ARGS:
              '--no-sandbox,--disable-setuid-sandbox,--disable-gpu,--use-mock-keychain,--password-store=basic'
          }
        : {})
    }
    for (const key of [
      'SSH_AUTH_SOCK',
      'DBUS_SESSION_BUS_ADDRESS',
      'BROWSER_CDP_URL',
      'BU_CDP_URL',
      'BU_CDP_WS',
      'CAMOFOX_URL',
      'AGENT_BROWSER_EXECUTABLE_PATH',
      'XDG_RUNTIME_DIR'
    ])
      delete env[key]
    return { ...launch, cwd, env, cleanup }
  } catch (error) {
    cleanup()
    throw error
  }
}
