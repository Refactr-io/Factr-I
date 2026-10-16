import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import { findReadyZip, readPostUpdateSync, writeHealthMarker } from './update-launch'

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
