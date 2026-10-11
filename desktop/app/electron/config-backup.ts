/**
 * config-backup.ts
 *
 * "Back up and start fresh" for a config.yaml the engine cannot read (QA D-3).
 * The broken file is COPIED to `config.yaml.corrupt-<timestamp>` next to it,
 * the copy is checked byte for byte, and only then is the original removed, so
 * a fresh setup can never overwrite it and nothing is lost. Pure apart from the
 * injected fs, so it is unit-tested without Electron.
 */

import nodeFs from 'node:fs'
import path from 'node:path'

type BackupFs = Pick<typeof nodeFs, 'constants' | 'copyFileSync' | 'lstatSync' | 'readFileSync' | 'realpathSync' | 'unlinkSync'>

export type ConfigBackupResult = { backupPath: string; ok: true } | { message: string; ok: false }

/** `2026-10-11T05-48-07-123Z`: sortable, and safe in a file name on every OS. */
export function corruptBackupStamp(now: number): string {
  return new Date(now).toISOString().replace(/[:.]/g, '-')
}

function insideRoot(target: string, root: string): boolean {
  const rel = path.relative(root, target)

  return Boolean(rel) && !rel.startsWith('..') && !path.isAbsolute(rel)
}

/**
 * Back up `configPath` and remove it. Refuses anything that is not a regular
 * `config.yaml` under `configRoot` (the config home this desktop runs with), so
 * the renderer cannot point this at another file.
 */
export function backupCorruptConfig(
  configPath: string,
  { configRoot, fs = nodeFs, now = Date.now() }: { configRoot: string; fs?: BackupFs; now?: number }
): ConfigBackupResult {
  const target = path.resolve(String(configPath || ''))

  if (path.basename(target) !== 'config.yaml') {
    return { ok: false, message: `Not a Factr-I config file: ${target}` }
  }

  let real: string

  try {
    if (!fs.lstatSync(target).isFile()) {
      return { ok: false, message: `${target} is not a regular file (a link or a folder). Fix or move it yourself.` }
    }

    real = fs.realpathSync(target)
  } catch (error) {
    return { ok: false, message: `${target} cannot be read: ${(error as Error).message}` }
  }

  let root: string

  try {
    root = fs.realpathSync(path.resolve(configRoot))
  } catch {
    root = path.resolve(configRoot)
  }

  if (!insideRoot(real, root)) {
    return { ok: false, message: `${target} is outside the Factr-I config folder (${configRoot}).` }
  }

  const backupPath = `${target}.corrupt-${corruptBackupStamp(now)}`

  try {
    // COPYFILE_EXCL: never replace an earlier backup.
    fs.copyFileSync(target, backupPath, fs.constants.COPYFILE_EXCL)

    if (!fs.readFileSync(backupPath).equals(fs.readFileSync(target))) {
      return { ok: false, message: `The backup ${backupPath} does not match ${target}; nothing was removed.` }
    }

    fs.unlinkSync(target)
  } catch (error) {
    return { ok: false, message: `Could not back up ${target}: ${(error as Error).message}` }
  }

  return { ok: true, backupPath }
}
