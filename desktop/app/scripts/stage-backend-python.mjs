// Build-time only: make forwarded Factr features work without a user Python install.
import { cpSync, existsSync, readdirSync, readlinkSync, mkdirSync, realpathSync, rmSync, copyFileSync, writeFileSync } from 'node:fs'
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

// Fail the build on any symlink that is absolute or points outside `root` (breaks codesign and relocation).
export function badSymlinks(root) {
  const bad = []
  const walk = dir => {
    for (const e of readdirSync(dir, { withFileTypes: true })) {
      const p = path.join(dir, e.name)
      if (e.isSymbolicLink()) {
        const t = readlinkSync(p)
        const rel = path.relative(root, path.resolve(dir, t))
        if (path.isAbsolute(t) || rel.startsWith('..') || path.isAbsolute(rel)) bad.push(`${path.relative(root, p)} -> ${t}`)
      } else if (e.isDirectory()) walk(p)
    }
  }
  walk(root)
  return bad
}
if (process.argv[2] === '--check') {
  const bad = badSymlinks(path.resolve(process.argv[3]))
  if (bad.length) { console.error(`Bad symlinks:\n${bad.join('\n')}`); process.exit(1) }
  process.exit(0)
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
cpSync(runtime, path.join(stage, 'runtime'), { recursive: true, verbatimSymlinks: true })
const stagedPython = path.join(stage, 'runtime', target.pythonRel)
if (process.env.FACTR_BACKEND_PYTHON_PACKAGES) {
  cpSync(packages, path.join(stage, 'packages'), { recursive: true, verbatimSymlinks: true })
} else {
  execFileSync(
    'uv',
    ['pip', 'install', '--target', path.join(stage, 'packages'), '--python', stagedPython, '--requirements', path.join(backend, 'pyproject.toml'), '--extra', 'bundled'],
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
// Prove the staged tree imports the bundled optional libraries (docs/BUNDLED-EXTRAS.md), so a missing
// package fails at staging, not on a user's machine. Windows: PYTHONPATH entries get no .pth processing,
// so also import the pywin32-backed modules (factr_bootstrap runs site.addsitedir on PYTHONPATH first).
const probeModules = [
  'telegram', 'telegram.ext', 'discord', 'nacl', 'aiohttp', 'brotlicffi', 'slack_bolt', 'slack_sdk', 'qrcode',
  'mautrix', 'dingtalk_stream', 'microsoft_teams.apps', 'defusedxml', 'anthropic', 'mcp', 'acp', 'exa_py',
  'firecrawl', 'parallel', 'fal_client', 'edge_tts', 'youtube_transcript_api',
]
const probe = process.platform === 'win32'
  ? ['factr_bootstrap', 'pywintypes', 'win32file', 'factr_logging', 'factr_backend.main', ...probeModules]
  : ['factr_bootstrap', ...probeModules]
execFileSync(stagedPython, ['-c', `import ${probe.join(', ')}`], {
  stdio: 'inherit',
  env: { ...process.env, PYTHONPATH: [source, path.join(stage, 'packages')].join(path.delimiter) }
})
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
const offenders = badSymlinks(stage)
if (offenders.length) throw new Error(`Staged tree has absolute or escaping symlinks:\n${offenders.join('\n')}`)
console.log(`Staged Factr Python runtime at ${stage} (${key})`)
