// Post-build integrity check for an unpacked macOS app: run the bundled Python the way the app does
// (temp HOME, bytecode writes off), import the backend, then prove the bundle's signature still holds.
//   node scripts/verify-packaged-app.mjs <path/to/Factr-I.app>
// Never touches anything when no app path is given.
import { execFileSync, spawnSync } from 'node:child_process'
import { existsSync, mkdtempSync, rmSync } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { isMain } from './utils.mjs'

export const SMOKE_MODULES = ['factr_bootstrap', 'factr_constants', 'factr_backend.main', 'factr_backend.uninstall', 'agent.credential_sources']

export function bundledPython(appPath) {
  const root = path.join(appPath, 'Contents', 'Resources', 'backend-python')
  return {
    root,
    python: path.join(root, 'runtime', 'bin', 'python3.12'),
    pythonPath: [path.join(root, 'source'), path.join(root, 'packages')].join(path.delimiter)
  }
}

// The same environment main.ts hands the engine (and so every bundled-Python child), under a throwaway HOME.
export function smokeEnv(appPath, home) {
  const { root, pythonPath } = bundledPython(appPath)
  return {
    PATH: '/usr/bin:/bin:/usr/sbin:/sbin',
    HOME: home,
    TMPDIR: home,
    FACTR_CONFIG_HOME: path.join(home, '.factr'),
    FACTR_HOME: path.join(home, '.factr', 'engine'),
    XDG_CONFIG_HOME: path.join(home, '.config'),
    XDG_CACHE_HOME: path.join(home, '.cache'),
    PYTHONDONTWRITEBYTECODE: '1',
    PYTHONPATH: pythonPath,
    FACTR_BACKEND_DEFAULTS: path.join(root, 'defaults.json'),
    FACTR_BACKEND_TOOLS_DIR: path.join(root, 'tools'),
    NO_COLOR: '1'
  }
}

export function parseArgs(argv) {
  const appPath = argv[0]
  if (!appPath) return { error: 'usage: node scripts/verify-packaged-app.mjs <path/to/Factr-I.app>' }
  if (!appPath.endsWith('.app')) return { error: `not an .app bundle: ${appPath}` }
  return { appPath: path.resolve(appPath) }
}

function main() {
  const { appPath, error } = parseArgs(process.argv.slice(2))
  if (error) { console.error(error); process.exit(2) }
  if (process.platform !== 'darwin') { console.error('codesign verification needs macOS'); process.exit(2) }
  const { python } = bundledPython(appPath)
  if (!existsSync(python)) { console.error(`bundled Python missing: ${python}`); process.exit(1) }
  const home = mkdtempSync(path.join(os.tmpdir(), 'factr-verify-home-'))
  let failed = false
  try {
    const smoke = spawnSync(python, ['-c', `import ${SMOKE_MODULES.join(', ')}; print('import smoke ok')`], {
      cwd: home, env: smokeEnv(appPath, home), stdio: 'inherit', timeout: 120_000
    })
    if (smoke.status !== 0) { console.error(`import smoke failed (${smoke.status ?? smoke.signal})`); failed = true }
    // After running: the signature must still cover the bundle exactly (no __pycache__ or other writes).
    execFileSync('codesign', ['--verify', '--deep', '--strict', '--verbose=2', appPath], { stdio: 'inherit' })
    console.log(`codesign --verify --deep --strict: ok (${appPath})`)
  } catch (err) {
    console.error(`codesign verification failed: ${err.message}`)
    failed = true
  } finally {
    rmSync(home, { recursive: true, force: true })
  }
  process.exit(failed ? 1 : 0)
}

if (isMain(import.meta.url)) main()
