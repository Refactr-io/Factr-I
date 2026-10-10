// Build-time only: make forwarded Factr features work without a user Python install.
import { chmodSync, cpSync, existsSync, lstatSync, readdirSync, readFileSync, readlinkSync, mkdirSync, realpathSync, rmSync, copyFileSync, writeFileSync } from 'node:fs'
import { execFileSync, spawnSync } from 'node:child_process'
import path from 'node:path'
import { isMain } from './utils.mjs'

const repo = path.resolve(import.meta.dirname, '../../..')
const backend = path.join(repo, 'backend')
const stage = path.resolve(import.meta.dirname, '../build/backend-python')
const managed = path.resolve(import.meta.dirname, '../build/backend-python-managed')
// Kept next to (not inside) the staged tree so it never ships in the app bundle.
export const integrityPath = root => `${root}.integrity.json`
const version = '3.12.12'
const pycTag = 'cpython-312'

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

function walkFiles(root, visit) {
  const walk = dir => {
    for (const e of readdirSync(dir, { withFileTypes: true })) {
      const p = path.join(dir, e.name)
      if (e.isDirectory()) walk(p)
      else visit(p, e)
    }
  }
  walk(root)
}

// Fail the build on any symlink that is absolute or points outside `root` (breaks codesign and relocation).
export function badSymlinks(root) {
  const bad = []
  walkFiles(root, (p, e) => {
    if (!e.isSymbolicLink()) return
    const t = readlinkSync(p)
    const rel = path.relative(root, path.resolve(path.dirname(p), t))
    if (path.isAbsolute(t) || rel.startsWith('..') || path.isAbsolute(rel)) bad.push(`${path.relative(root, p)} -> ${t}`)
  })
  return bad
}

// Read-only files (uv ships 0555, some wheels 0444) make `xattr -cr` and re-signing fail.
export function makeUserWritable(root) {
  const fix = p => {
    const st = lstatSync(p)
    if (!st.isSymbolicLink() && !(st.mode & 0o200)) chmodSync(p, st.mode | 0o200)
  }
  fix(root)
  const walk = dir => {
    for (const e of readdirSync(dir, { withFileTypes: true })) {
      const p = path.join(dir, e.name)
      fix(p)
      if (e.isDirectory()) walk(p)
    }
  }
  walk(root)
}

// The pyc Python 3.12 looks for for `file` (no -O): `<dir>/__pycache__/<name>.cpython-312.pyc`.
export function pycFor(file) {
  return path.join(path.dirname(file), '__pycache__', `${path.basename(file, '.py')}.${pycTag}.pyc`)
}

// Relative paths of the .py files under `root` that have no precompiled pyc.
export function sourcesMissingPyc(root) {
  const missing = []
  walkFiles(root, (p, e) => {
    if (e.isFile() && p.endsWith('.py') && !existsSync(pycFor(p))) missing.push(path.relative(root, p))
  })
  return missing.sort()
}

/** Snapshot of the staged tree, taken last: anything that writes into it later (an import that
 *  drops __pycache__, a stray edit) shows up as a newer mtime, a changed file count, or a new pyc gap. */
export function snapshotIntegrity(root, stampedAt = Date.now()) {
  let files = 0
  let newest = 0
  walkFiles(root, (p, e) => {
    if (e.isSymbolicLink()) return
    files += 1
    newest = Math.max(newest, lstatSync(p).mtimeMs)
  })
  return { stampedAt: Math.max(stampedAt, Math.ceil(newest)), files, uncompilable: sourcesMissingPyc(root) }
}

// Problems with `root` against its stage-time snapshot; empty means the tree is as staged.
export function verifyIntegrity(root, snapshot) {
  const problems = []
  const changed = []
  let files = 0
  walkFiles(root, (p, e) => {
    if (e.isSymbolicLink()) return
    files += 1
    if (lstatSync(p).mtimeMs > snapshot.stampedAt) changed.push(path.relative(root, p))
  })
  if (changed.length) problems.push(`${changed.length} file(s) modified after staging, e.g. ${changed.slice(0, 5).join(', ')}`)
  if (files !== snapshot.files) problems.push(`file count changed after staging: ${snapshot.files} -> ${files}`)
  const allowed = new Set(snapshot.uncompilable)
  const missing = sourcesMissingPyc(root).filter(f => !allowed.has(f))
  if (missing.length) problems.push(`${missing.length} source file(s) without a precompiled pyc, e.g. ${missing.slice(0, 5).join(', ')}`)
  return problems
}

// The bundle is signed and read-only once installed, so pycs never need revalidating against
// source; unchecked-hash pycs are used as-is (no per-start stat or rewrite). `-s` keeps the build
// machine's path out of the code objects.
export function compileallArgs(root, dirs) {
  return ['-m', 'compileall', '-q', '-f', '-j', '0', '--invalidation-mode', 'unchecked-hash', '-s', root, ...dirs.map(d => path.join(root, d))]
}

// The backend test suite (~50 MB, thousands of files) is not needed at runtime.
export function archiveArgs() {
  return ['archive', '--format=tar', 'HEAD:backend', ':(exclude)tests']
}

/** Desktop-packaged defaults differ from the CLI's in one place: never borrow the Claude Code /
 *  Codex CLI logins (refreshing a borrowed single-use token logs the other tool out). */
export function packagedDefaults(defaults) {
  const out = { ...defaults }
  out.auth = { ...(defaults.auth || {}), adopt_external_logins: false }
  return out
}

function main() {
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
  rmSync(integrityPath(stage), { force: true })
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
  const archive = execFileSync('git', archiveArgs(), { cwd: repo, maxBuffer: 256 * 1024 * 1024 })
  const extracted = spawnSync('tar', ['-xf', '-', '-C', source], { input: archive, stdio: ['pipe', 'inherit', 'inherit'] })
  if (extracted.status !== 0) throw new Error('Could not stage Factr source')
  const pyEnv = { ...process.env, PYTHONDONTWRITEBYTECODE: '1', PYTHONPATH: [source, path.join(stage, 'packages')].join(path.delimiter) }
  const defaults = execFileSync(stagedPython, [
    '-c', 'import json; from factr_backend.config_defaults import DEFAULT_CONFIG; print(json.dumps(DEFAULT_CONFIG))'
  ], { env: pyEnv, encoding: 'utf8' })
  writeFileSync(path.join(stage, 'defaults.json'), JSON.stringify(packagedDefaults(JSON.parse(defaults))))
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
  execFileSync(stagedPython, ['-c', `import ${probe.join(', ')}`], { stdio: 'inherit', env: pyEnv })
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
  // Precompile before signing: a signed bundle in a read-only /Applications cannot cache bytecode, so
  // without this every start recompiles (and a writable one writes thousands of pycs into its own signature).
  makeUserWritable(stage)
  const compiled = spawnSync(stagedPython, compileallArgs(stage, ['runtime', 'packages', 'source']), {
    cwd: stage, stdio: ['ignore', 'ignore', 'inherit'], env: { ...process.env, PYTHONPATH: '' }
  })
  // compileall exits 1 when a few files (test data with deliberate syntax errors) do not compile; those
  // are recorded below as allowed gaps. Anything else is a broken runtime.
  if (compiled.status !== 0 && compiled.status !== 1) throw new Error(`compileall failed (${compiled.status})`)
  makeUserWritable(stage)
  const snapshot = snapshotIntegrity(stage)
  writeFileSync(integrityPath(stage), JSON.stringify(snapshot))
  console.log(`Staged Factr Python runtime at ${stage} (${key}); ${snapshot.files} files, ${snapshot.uncompilable.length} source(s) without pyc`)
}

// `--check <dir>`: symlinks only. `--verify <dir>`: symlinks plus the stage-time integrity snapshot.
function cli(mode, dir) {
  const root = path.resolve(dir)
  const problems = badSymlinks(root).map(b => `bad symlink ${b}`)
  if (mode === '--verify') {
    const snap = integrityPath(root)
    if (!existsSync(snap)) problems.push(`no integrity snapshot at ${snap}; run npm run stage:backend-python`)
    else problems.push(...verifyIntegrity(root, JSON.parse(readFileSync(snap, 'utf8'))))
  }
  if (problems.length) { console.error(`Staged backend-python is not clean:\n${problems.join('\n')}`); process.exit(1) }
  process.exit(0)
}

if (isMain(import.meta.url)) {
  if (process.argv[2] === '--check' || process.argv[2] === '--verify') cli(process.argv[2], process.argv[3])
  else main()
}
