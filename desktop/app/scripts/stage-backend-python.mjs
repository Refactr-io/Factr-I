// Build-time only: make forwarded Factr features work without a user Python install.
import { cpSync, existsSync, mkdirSync, realpathSync, rmSync, copyFileSync, writeFileSync } from 'node:fs'
import { execFileSync, spawnSync } from 'node:child_process'
import path from 'node:path'

const repo = path.resolve(import.meta.dirname, '../../..')
const backend = path.join(repo, 'backend')
const stage = path.resolve(import.meta.dirname, '../build/backend-python')
const managed = path.resolve(import.meta.dirname, '../build/backend-python-managed')
const version = '3.12.12'

const targets = {
  'darwin-arm64': {
    runtimeDir: `cpython-${version}-macos-aarch64-none`,
    pythonRel: path.join('bin', 'python3.12'),
    uvName: 'uv',
  },
  'darwin-x64': {
    runtimeDir: `cpython-${version}-macos-x86_64-none`,
    pythonRel: path.join('bin', 'python3.12'),
    uvName: 'uv',
  },
  'linux-x64': {
    runtimeDir: `cpython-${version}-linux-x86_64-gnu`,
    pythonRel: path.join('bin', 'python3.12'),
    uvName: 'uv',
  },
  'win32-x64': {
    runtimeDir: `cpython-${version}-windows-x86_64-none`,
    pythonRel: 'python.exe',
    uvName: 'uv.exe',
  },
}

const key = `${process.platform}-${process.arch}`
const target = targets[key]
if (!target) {
  throw new Error(
    `Python staging supports macOS arm64/x64, Linux x64 and Windows x64 only (got ${key})`
  )
}

const runtime = process.env.FACTR_BACKEND_PYTHON_RUNTIME || path.join(managed, target.runtimeDir)
const packages = process.env.FACTR_BACKEND_PYTHON_PACKAGES || path.join(stage, 'packages')
if (!process.env.FACTR_BACKEND_PYTHON_RUNTIME) {
  execFileSync('uv', ['python', 'install', version, '--install-dir', managed], { stdio: 'inherit' })
}
const pythonBin = path.join(runtime, target.pythonRel)
if (!existsSync(pythonBin)) throw new Error(`Python runtime missing: ${pythonBin}`)
rmSync(stage, { recursive: true, force: true })
mkdirSync(stage, { recursive: true })
cpSync(runtime, path.join(stage, 'runtime'), { recursive: true })
const stagedPython = path.join(stage, 'runtime', target.pythonRel)
if (process.env.FACTR_BACKEND_PYTHON_PACKAGES) {
  cpSync(packages, path.join(stage, 'packages'), { recursive: true })
} else {
  execFileSync(
    'uv',
    ['pip', 'install', '--target', path.join(stage, 'packages'), '--python', stagedPython, '--requirements', path.join(backend, 'pyproject.toml')],
    { cwd: repo, stdio: 'inherit' }
  )
}

const source = path.join(stage, 'source')
mkdirSync(source)
const archive = execFileSync('git', ['archive', '--format=tar', 'HEAD:backend'], { cwd: repo, maxBuffer: 256 * 1024 * 1024 })
const extracted = spawnSync('tar', ['-xf', '-', '-C', source], { input: archive, stdio: ['pipe', 'inherit', 'inherit'] })
if (extracted.status !== 0) throw new Error('Could not stage Factr source')
const defaults = execFileSync(stagedPython, [
  '-c', 'import json; from factr_backend.config_defaults import DEFAULT_CONFIG; print(json.dumps(DEFAULT_CONFIG))'
], { env: { ...process.env, PYTHONPATH: [source, path.join(stage, 'packages')].join(path.delimiter) } })
writeFileSync(path.join(stage, 'defaults.json'), defaults)
// Windows: PYTHONPATH entries get no .pth processing, so prove the staged tree imports pywin32-backed modules
// (factr_bootstrap runs site.addsitedir on PYTHONPATH first). Other platforms keep the probe above.
if (process.platform === 'win32') {
  execFileSync(stagedPython, ['-c', 'import factr_bootstrap, pywintypes, win32file, factr_logging, factr_backend.main'], {
    stdio: 'inherit',
    env: { ...process.env, PYTHONPATH: [source, path.join(stage, 'packages')].join(path.delimiter) }
  })
}
const tools = path.join(stage, 'tools')
mkdirSync(tools)
const uvPath = process.platform === 'win32'
  ? execFileSync('where', ['uv'], { encoding: 'utf8' }).trim().split(/\r?\n/)[0]
  : execFileSync('which', ['uv'], { encoding: 'utf8' }).trim()
copyFileSync(realpathSync(uvPath), path.join(tools, target.uvName))
// Inputs to the pack-time version manifest (scripts/factr-manifest.mjs).
const git = args => execFileSync('git', args, { cwd: repo, encoding: 'utf8' }).trim()
writeFileSync(
  path.join(stage, 'stage.json'),
  JSON.stringify({ factrSha: git(['rev-parse', 'HEAD']), factrDirty: git(['status', '--porcelain', '-uno']) !== '', pythonVersion: version })
)
console.log(`Staged Factr Python runtime at ${stage} (${key})`)
