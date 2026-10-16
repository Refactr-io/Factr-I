import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

import { describe, expect, it } from 'vitest'

import { readAutoDownload, writeAutoDownload } from './update-download-pref'

describe('auto-download preference', () => {
  const file = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'factr-pref-')), 'sub', 'auto.json')

  it('defaults to ON when nothing is saved or the file is unreadable', () => {
    expect(readAutoDownload(file)).toBe(true)
    fs.mkdirSync(path.dirname(file), { recursive: true })
    fs.writeFileSync(file, '{not json', 'utf8')
    expect(readAutoDownload(file)).toBe(true)
  })

  it('persists off and back on', () => {
    writeAutoDownload(file, false)
    expect(readAutoDownload(file)).toBe(false)
    writeAutoDownload(file, true)
    expect(readAutoDownload(file)).toBe(true)
  })
})
