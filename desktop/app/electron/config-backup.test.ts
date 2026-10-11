import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

import { afterEach, beforeEach, describe, expect, it } from 'vitest'

import { backupCorruptConfig, corruptBackupStamp } from './config-backup'

const BROKEN = 'model:\n  provider: custom\n  default: [stub-model\n  base_url: : :\n'
const NOW = Date.UTC(2026, 9, 11, 5, 48, 7, 123)

describe('backupCorruptConfig (D-3: back up and start fresh)', () => {
  let root: string

  beforeEach(() => {
    root = fs.mkdtempSync(path.join(os.tmpdir(), 'factr config '))
  })

  afterEach(() => {
    fs.rmSync(root, { force: true, recursive: true })
  })

  it('copies the broken file next to itself before removing it', () => {
    const config = path.join(root, 'config.yaml')
    fs.writeFileSync(config, BROKEN)

    const result = backupCorruptConfig(config, { configRoot: root, now: NOW })

    expect(result).toEqual({ ok: true, backupPath: `${config}.corrupt-2026-10-11T05-48-07-123Z` })
    expect(fs.existsSync(config)).toBe(false)
    expect(fs.readFileSync(`${config}.corrupt-${corruptBackupStamp(NOW)}`, 'utf8')).toBe(BROKEN)
  })

  it('works for a named profile under the config home', () => {
    const config = path.join(root, 'profiles', 'coder', 'config.yaml')
    fs.mkdirSync(path.dirname(config), { recursive: true })
    fs.writeFileSync(config, BROKEN)

    expect(backupCorruptConfig(config, { configRoot: root, now: NOW }).ok).toBe(true)
  })

  it('never replaces an earlier backup and then keeps the original', () => {
    const config = path.join(root, 'config.yaml')
    fs.writeFileSync(config, BROKEN)
    fs.writeFileSync(`${config}.corrupt-${corruptBackupStamp(NOW)}`, 'older backup')

    const result = backupCorruptConfig(config, { configRoot: root, now: NOW })

    expect(result.ok).toBe(false)
    expect(fs.readFileSync(config, 'utf8')).toBe(BROKEN)
    expect(fs.readFileSync(`${config}.corrupt-${corruptBackupStamp(NOW)}`, 'utf8')).toBe('older backup')
  })

  it('refuses another file, a file outside the config home, and a link', () => {
    const other = path.join(root, '.env')
    fs.writeFileSync(other, 'KEY=fake')
    expect(backupCorruptConfig(other, { configRoot: root }).ok).toBe(false)
    expect(fs.existsSync(other)).toBe(true)

    const outsideDir = fs.mkdtempSync(path.join(os.tmpdir(), 'elsewhere-'))
    const outside = path.join(outsideDir, 'config.yaml')
    fs.writeFileSync(outside, BROKEN)

    try {
      expect(backupCorruptConfig(outside, { configRoot: root }).ok).toBe(false)
      expect(fs.existsSync(outside)).toBe(true)

      if (process.platform !== 'win32') {
        const link = path.join(root, 'config.yaml')
        fs.symlinkSync(outside, link)
        expect(backupCorruptConfig(link, { configRoot: root }).ok).toBe(false)
        expect(fs.existsSync(outside)).toBe(true)
      }
    } finally {
      fs.rmSync(outsideDir, { force: true, recursive: true })
    }
  })
})
