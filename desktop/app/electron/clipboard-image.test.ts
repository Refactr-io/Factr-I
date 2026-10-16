import assert from 'node:assert/strict'

import { test } from 'vitest'

import { type ClipboardLike, readClipboardImage } from './clipboard-image'

const PNG = Buffer.concat([Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]), Buffer.from('payload')])

const fake = (formats: Record<string, Buffer>, bitmap?: Buffer): ClipboardLike => ({
  availableFormats: () => Object.keys(formats),
  readBuffer: format => formats[format] ?? Buffer.alloc(0),
  readImage: () => ({ isEmpty: () => !bitmap, toPNG: () => bitmap ?? Buffer.alloc(0) })
})

test('a plain bitmap wins', () => {
  const got = readClipboardImage(fake({}, PNG))

  assert.equal(got?.source, 'bitmap')
  assert.equal(got?.ext, '.png')
})

test('a raw PNG clipboard format is read when the bitmap is empty (Windows screenshot tools)', () => {
  const got = readClipboardImage(fake({ PNG }))

  assert.equal(got?.source, 'png-format')
  assert.deepEqual(got?.data, PNG)
})

test('a non-PNG buffer under the PNG format is ignored', () => {
  assert.equal(readClipboardImage(fake({ PNG: Buffer.from('not an image at all') })), null)
})

test('a copied image file (Windows FileNameW) is read from disk', () => {
  const name = Buffer.from('C:\\Users\\me\\Pictures\\shot.JPG\0', 'utf16le')

  const got = readClipboardImage(fake({ FileNameW: name }), {
    readFile: file => {
      assert.equal(file, 'C:\\Users\\me\\Pictures\\shot.JPG')

      return Buffer.from('jpeg-bytes')
    },
    stat: () => ({ size: 10 })
  })

  assert.equal(got?.source, 'file')
  assert.equal(got?.ext, '.jpg')
})

test('a copied non-image file is not an image', () => {
  const name = Buffer.from('C:\\Users\\me\\notes.txt\0', 'utf16le')

  assert.equal(readClipboardImage(fake({ FileNameW: name }), { stat: () => ({ size: 10 }) }), null)
})

test('a macOS copied image file url is read', () => {
  const url = Buffer.from('file:///Users/me/My%20Pics/a.png', 'utf8')

  const got = readClipboardImage(fake({ 'public.file-url': url }), {
    readFile: file => {
      assert.equal(file, '/Users/me/My Pics/a.png')

      return PNG
    },
    stat: () => ({ size: 12 })
  })

  assert.equal(got?.source, 'file')
})

test('nothing on the clipboard is null', () => {
  assert.equal(readClipboardImage(fake({})), null)
})
