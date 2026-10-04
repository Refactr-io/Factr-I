// One version manifest for the three parts that ship together: the Factr engine binary, the bundled
// Factr source and the Python runtime. Written into build/factr/ (-> resources/factr/manifest.json)
// at pack time; electron/factr-manifest.ts checks the running engine against it.
import { createHash } from 'node:crypto'
import { execFileSync } from 'node:child_process'
import { existsSync, readFileSync, writeFileSync } from 'node:fs'
import path from 'node:path'

/** `{version, sha}` from the binary's own `__version`; null when it can't run here (cross-arch, old build). */
export function probeEngine(binary, exec = execFileSync) {
  try {
    const { version, sha, db_schema: dbSchema } = JSON.parse(exec(binary, ['__version'], { encoding: 'utf8', timeout: 10_000 }))
    if (typeof version !== 'string') return null
    // dbSchema (factr.db migrations the engine runs) lets factr-update.sh wait longer for a first start that migrates.
    return { version, sha: typeof sha === 'string' && sha ? sha : null, ...(Number.isInteger(dbSchema) ? { dbSchema } : {}) }
  } catch {
    return null
  }
}

export function buildManifest({ engine, stage, desktopVersion, builtAt = new Date().toISOString() }) {
  const body = {
    schema: 1,
    builtAt,
    desktop: desktopVersion,
    engine: engine ?? { version: null, sha: null },
    factr: { sha: stage.factrSha ?? null, dirty: Boolean(stage.factrDirty) },
    python: { version: stage.pythonVersion }
  }
  const id = createHash('sha256').update(JSON.stringify(body)).digest('hex').slice(0, 16)
  return { id, ...body }
}

/**
 * Write `<outDir>/manifest.json`. `stagePath` is build/backend-python/stage.json (written by
 * stage-backend-python.mjs); a missing one means the runtime was not staged, which is a broken pack.
 * `canRun` is true when `binary` matches this machine, so it can be asked for its version; otherwise
 * `engineRoot`'s Cargo.toml gives the version and the sha is left unchecked.
 */
export function writeFactrManifest({ binary, canRun, engineRoot, stagePath, outDir, desktopVersion }) {
  if (!existsSync(stagePath)) throw new Error('Run npm run stage:backend-python before packaging (stage.json missing)')
  const stage = JSON.parse(readFileSync(stagePath, 'utf8'))
  let engine = canRun ? probeEngine(binary) : null
  if (!engine) {
    console.warn(`[factr-manifest] could not ask ${binary} for its version (cross-arch or older build); the app will check the engine version only`)
    const cargo = readFileSync(path.join(engineRoot, 'Cargo.toml'), 'utf8')
    engine = { version: /^version\s*=\s*"([^"]+)"/m.exec(cargo)?.[1] ?? null, sha: null }
    if (!engine.version) throw new Error(`no engine version found in ${engineRoot}/Cargo.toml`)
  }
  // Checked by factr-update.sh with `shasum -a 256` before the bundle replaces the installed one.
  engine = { ...engine, sha256: createHash('sha256').update(readFileSync(binary)).digest('hex') }
  const manifest = buildManifest({ engine, stage, desktopVersion })
  writeFileSync(path.join(outDir, 'manifest.json'), JSON.stringify(manifest, null, 2) + '\n')
  return manifest
}
