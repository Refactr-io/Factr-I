import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import { cleanupUpdateLeftovers, findReadyInstaller, findReadyZip, readPostUpdateSync, writeHealthMarker } from './update-launch'

const dirs: string[] = []

const mk = () => {
  const d = fs.mkdtempSync(path.join(os.tmpdir(), 'upl-'))
  dirs.push(d)

  return d
}

afterEach(() => {
  for (const d of dirs.splice(0)) {
    fs.rmSync(d, { recursive: true, force: true })
  }
})

const put = (root: string, version: string, name = `Factr-I-${version}-mac-arm64.zip`) => {
  fs.mkdirSync(path.join(root, version), { recursive: true })
  fs.writeFileSync(path.join(root, version, name), 'x')
}

describe('findReadyZip', () => {
  it('returns the newest verified zip newer than the running version', () => {
    const root = mk()

    put(root, '0.0.8')
    put(root, '0.0.9')
    put(root, '0.0.10')

    expect(findReadyZip(root, '0.0.8')).toEqual({ file: path.join(root, '0.0.10', 'Factr-I-0.0.10-mac-arm64.zip'), kind: 'zip', version: '0.0.10' })
    expect(findReadyZip(root, '0.0.10')).toBeNull()
  })

  it('ignores partial downloads, other files and a version that was rolled back', () => {
    const root = mk()

    put(root, '0.0.9', 'Factr-I-0.0.9-mac-arm64.zip.partial')
    expect(findReadyZip(root, '0.0.8')).toBeNull()
    put(root, '0.0.9')
    fs.writeFileSync(path.join(root, '0.0.9', 'rolledback'), '')
    expect(findReadyZip(root, '0.0.8')).toBeNull()
    expect(findReadyZip(path.join(root, 'missing'), '0.0.8')).toBeNull()
  })
})

describe('post-update record and health marker', () => {
  it('reads the record only for the running version and writes the marker', () => {
    const root = mk()
    const marker = path.join(root, '0.0.9', 'ok')

    fs.writeFileSync(path.join(root, 'post-update.json'), JSON.stringify({ from: '0.0.8', to: '0.0.9', marker }))
    expect(readPostUpdateSync(root, '0.0.8')).toBeNull()

    const record = readPostUpdateSync(root, '0.0.9')

    expect(record).toEqual({ from: '0.0.8', to: '0.0.9', marker })
    writeHealthMarker(record!)
    expect(fs.existsSync(marker)).toBe(true)
  })
})

describe('retry guards and leftovers', () => {
  it('skips versions that were attempted, skipped or rolled back', () => {
    for (const marker of ['attempted', 'skipped', 'rolledback']) {
      const root = mk()

      put(root, '0.0.9')
      fs.writeFileSync(path.join(root, '0.0.9', marker), '')
      expect(findReadyZip(root, '0.0.8')).toBeNull()
    }
  })

  it('removes the old copy and old staging folders but keeps a staging folder for a newer version', () => {
    const apps = mk()
    const app = path.join(apps, 'Factr-I.app')

    fs.mkdirSync(app)

    for (const name of ['.Factr-I.app.old', '.Factr-I.app.failed', '.Factr-I-0.0.9.staging', '.Factr-I-0.0.10.staging', 'Other.app']) {
      fs.mkdirSync(path.join(apps, name))
    }

    cleanupUpdateLeftovers(app, '0.0.9')
    expect(fs.readdirSync(apps).sort()).toEqual(['.Factr-I-0.0.10.staging', 'Factr-I.app', 'Other.app'])
  })
})

describe('findReadyInstaller', () => {
  it('finds the verified installer for each platform and skips versions that were tried', () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'ready-'))

    try {
      const put = (version: string, name: string, ...markers: string[]) => {
        fs.mkdirSync(path.join(dir, version), { recursive: true })
        fs.writeFileSync(path.join(dir, version, name), 'x')

        for (const marker of markers) {
          fs.writeFileSync(path.join(dir, version, marker), '')
        }
      }

      put('0.0.14', 'Factr-I-0.0.14-win-x64.exe')
      put('0.0.15', 'Factr-I-0.0.15-win-x64.exe', 'rolledback')
      put('0.0.14', 'Factr-I-0.0.14-linux-x86_64.AppImage')
      put('0.0.14', 'Factr-I-0.0.14-mac-arm64.dmg')

      expect(findReadyInstaller(dir, '0.0.13', 'win32', 'x64')).toMatchObject({ kind: 'exe', version: '0.0.14' })
      expect(findReadyInstaller(dir, '0.0.13', 'linux', 'x64')).toMatchObject({ kind: 'appimage', version: '0.0.14' })
      expect(findReadyInstaller(dir, '0.0.13', 'darwin', 'arm64')).toBeNull()
      expect(findReadyInstaller(dir, '0.0.14', 'win32', 'x64')).toBeNull()
    } finally {
      fs.rmSync(dir, { recursive: true, force: true })
    }
  })
})
