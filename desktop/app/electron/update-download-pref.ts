// The "Download updates automatically" setting: one JSON flag in userData, default ON.

import fs from 'node:fs'
import path from 'node:path'

export function readAutoDownload(file: string): boolean {
  try {
    return JSON.parse(fs.readFileSync(file, 'utf8')).enabled !== false
  } catch {
    return true
  }
}

export function writeAutoDownload(file: string, enabled: boolean): void {
  try {
    fs.mkdirSync(path.dirname(file), { recursive: true })
    fs.writeFileSync(`${file}.tmp`, JSON.stringify({ enabled: enabled !== false }), 'utf8')
    fs.renameSync(`${file}.tmp`, file)
  } catch {
    // A pref that cannot be saved falls back to the default next launch; never fail the UI over it.
  }
}
