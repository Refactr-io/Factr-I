import crypto from 'node:crypto'
import os from 'node:os'
import path from 'node:path'

interface RuntimeDirOptions {
  env?: NodeJS.ProcessEnv
  tmpdir?: string
  uid?: number
  pathModule?: typeof path.posix
}

/**
 * The engine keeps its daemon lock and sockets in its runtime dir (`FACTR_RUNTIME_DIR`, else
 * `XDG_RUNTIME_DIR`, else the per-user TMPDIR). A backend for a second profile started next to the
 * primary shared that dir and was refused with "Another factr server process is already running".
 * Each profile backend the pool spawns gets its own dir under the same base. The name is a short
 * hash, not the profile name: socket paths are capped near 104 bytes on macOS. The uid keeps two
 * users on a shared `/tmp` apart.
 */
export function profileRuntimeDir(profile: string, opts: RuntimeDirOptions = {}): string {
  const env = opts.env ?? process.env
  const pathModule = opts.pathModule ?? path
  const base = env.FACTR_RUNTIME_DIR || env.XDG_RUNTIME_DIR || opts.tmpdir || os.tmpdir()
  const uid = opts.uid ?? (typeof process.getuid === 'function' ? process.getuid() : 0)
  const id = crypto.createHash('sha256').update(`${uid}\0${profile}`).digest('hex').slice(0, 12)

  return pathModule.join(base, `factr-rt-${id}`)
}

/** The env entry a profile backend is spawned with (placed after the inherited env so it wins). */
export function profileBackendRuntimeEnv(profile: string, opts: RuntimeDirOptions = {}): { FACTR_RUNTIME_DIR: string } {
  return { FACTR_RUNTIME_DIR: profileRuntimeDir(profile, opts) }
}
