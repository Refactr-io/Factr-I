// Find an image on the system clipboard in every shape Windows and macOS put one there.
// `clipboard.readImage()` covers a plain bitmap (CF_DIB, NSImage); it comes back empty for the other two
// common shapes: apps that register a raw "PNG" clipboard format (Snipping Tool, ShareX, Paint.NET, many
// browsers), and a copied image FILE (Explorer Ctrl+C puts a file list, not a bitmap, on the clipboard).

import fs from 'node:fs'
import path from 'node:path'

export interface ClipboardLike {
  availableFormats: () => string[]
  readBuffer: (format: string) => Buffer
  readImage: () => { isEmpty: () => boolean; toPNG: () => Buffer }
}

export interface ClipboardImage {
  data: Buffer
  ext: string
  source: 'bitmap' | 'file' | 'png-format'
}

const PNG_SIGNATURE = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a])
const IMAGE_EXTENSIONS = new Set(['.png', '.jpg', '.jpeg', '.gif', '.webp', '.bmp', '.tif', '.tiff'])
const MAX_IMAGE_BYTES = 40 * 1024 * 1024

const safeBuffer = (clip: ClipboardLike, format: string): Buffer => {
  try {
    return clip.readBuffer(format)
  } catch {
    return Buffer.alloc(0)
  }
}

/** Windows CF_HDROP names its first file as UTF-16LE in the "FileNameW" format (NUL terminated). */
export function clipboardFilePaths(clip: ClipboardLike): string[] {
  const raw = safeBuffer(clip, 'FileNameW')

  if (raw.length >= 4) {
    return raw
      .toString('utf16le')
      .split('\0')
      .map(entry => entry.trim())
      .filter(Boolean)
  }

  // macOS file copies expose a file URL under public.file-url.
  const url = safeBuffer(clip, 'public.file-url').toString('utf8').replace(/\0+$/, '').trim()

  if (url.startsWith('file://')) {
    try {
      return [decodeURIComponent(new URL(url).pathname)]
    } catch {
      return []
    }
  }

  return []
}

export function readClipboardImage(
  clip: ClipboardLike,
  io: { readFile?: (file: string) => Buffer; stat?: (file: string) => { size: number } | null } = {}
): ClipboardImage | null {
  const bitmap = clip.readImage()

  if (bitmap && !bitmap.isEmpty()) {
    return { data: bitmap.toPNG(), ext: '.png', source: 'bitmap' }
  }

  for (const format of ['PNG', 'image/png']) {
    const png = safeBuffer(clip, format)

    if (png.length > PNG_SIGNATURE.length && png.subarray(0, PNG_SIGNATURE.length).equals(PNG_SIGNATURE)) {
      return { data: png, ext: '.png', source: 'png-format' }
    }
  }

  const readFile = io.readFile ?? (file => fs.readFileSync(file))
  const stat = io.stat ?? (file => (fs.existsSync(file) ? fs.statSync(file) : null))

  for (const file of clipboardFilePaths(clip)) {
    const ext = path.extname(file).toLowerCase()

    if (!IMAGE_EXTENSIONS.has(ext)) {
      continue
    }

    try {
      const info = stat(file)

      if (info && info.size > 0 && info.size <= MAX_IMAGE_BYTES) {
        return { data: readFile(file), ext: ext === '.jpeg' ? '.jpg' : ext, source: 'file' }
      }
    } catch {
      // Unreadable file: try the next one.
    }
  }

  return null
}
