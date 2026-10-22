// Stage the pinned rtk build next to the engine (build/factr/rtk[.exe] -> resources/factr/).
//   node scripts/stage-rtk.mjs [darwin|linux|win32] [arm64|x64] [outDir]   (defaults: this machine, build/factr)
// The `rtk-oss-*` assets are the Apache-2.0-only build; the plain `rtk-*` and `rtk-proxy-*` assets also
// carry a Business Source License engine and are not used. Every download is checked against the sha256
// pinned below AND against the release's own checksums.txt; any mismatch fails the build.
import { execFileSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { chmodSync, copyFileSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { isMain } from './utils.mjs'

export const RTK_VERSION = '1.0.1'
const RELEASE = `https://github.com/rtk-ai/rtk/releases/download/v${RTK_VERSION}`

/** platform-arch -> the pinned asset (hashes copied from the v1.0.1 checksums.txt). */
export const RTK_ASSETS = Object.freeze({
  'darwin-arm64': { asset: 'rtk-oss-aarch64-apple-darwin.tar.gz', sha256: '4a4d9d09656ff29423336aa506e680afc3f7b6dafe84f0194a1c9a523a3c80e1' },
  'darwin-x64': { asset: 'rtk-oss-x86_64-apple-darwin.tar.gz', sha256: '2cf4d6dd2cabccbc74aa6aba2a48e60aad85b1a31e3d740e6ca0b2fa0060b06a' },
  'linux-x64': { asset: 'rtk-oss-x86_64-unknown-linux-musl.tar.gz', sha256: 'a44e8ffc43981100b4ddf1710399a1649a6c874591aa6d0c0727fe6cbd983c41' },
  'win32-x64': { asset: 'rtk-oss-x86_64-pc-windows-msvc.zip', sha256: '0e53e2eca471e15fbd3636e64e6bc0b182cbc278ed0339c80450a2dd03f4cd6b' }
})

export const sha256 = bytes => createHash('sha256').update(bytes).digest('hex')

/** The hash `checksums.txt` lists for `asset`, or null. */
export function publishedHash(checksums, asset) {
  const line = checksums.split(/\r?\n/).map(l => l.trim().split(/\s+/)).find(([, name]) => name === asset)

  return line ? line[0].toLowerCase() : null
}

/** Throws unless `bytes` match the pin and the published checksum. */
export function verifyAsset({ bytes, pinned, published, asset }) {
  const actual = sha256(bytes)

  if (actual !== pinned) throw new Error(`${asset}: sha256 ${actual} does not match the pinned ${pinned}`)
  if (published !== pinned) throw new Error(`${asset}: checksums.txt lists ${published ?? 'nothing'}, the pin is ${pinned}`)
}

function find(dir, names) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name)

    if (entry.isDirectory()) {
      const hit = find(full, names)

      if (hit) return hit
    } else if (names.includes(entry.name)) {
      return full
    }
  }

  return null
}

async function download(url) {
  const res = await fetch(url, { redirect: 'follow' })

  if (!res.ok) throw new Error(`GET ${url}: ${res.status}`)

  return Buffer.from(await res.arrayBuffer())
}

/** Download, verify and unpack the rtk for `platform-arch` into `outDir` as rtk[.exe] plus RTK-LICENSE. */
export async function stageRtk({ platform, arch, outDir, fetchBytes = download }) {
  const pin = RTK_ASSETS[`${platform}-${arch}`]

  if (!pin) throw new Error(`no pinned rtk build for ${platform}-${arch}`)

  const [bytes, sums] = await Promise.all([fetchBytes(`${RELEASE}/${pin.asset}`), fetchBytes(`${RELEASE}/checksums.txt`)])
  verifyAsset({ bytes, pinned: pin.sha256, published: publishedHash(sums.toString('utf8'), pin.asset), asset: pin.asset })

  const work = mkdtempSync(path.join(os.tmpdir(), 'rtk-stage-'))

  try {
    const archive = path.join(work, pin.asset)
    writeFileSync(archive, bytes)
    // bsdtar (macOS, Windows) unpacks both formats; GNU tar handles the tarballs.
    execFileSync('tar', ['-xf', archive, '-C', work])
    const exe = platform === 'win32' ? 'rtk.exe' : 'rtk'
    const binary = find(work, [exe])
    const licence = find(work, ['LICENSE'])

    if (!binary || !licence) throw new Error(`${pin.asset}: expected ${exe} and LICENSE in the archive`)
    mkdirSync(outDir, { recursive: true })
    copyFileSync(binary, path.join(outDir, exe))
    chmodSync(path.join(outDir, exe), 0o755)
    copyFileSync(licence, path.join(outDir, 'RTK-LICENSE'))
  } finally {
    rmSync(work, { recursive: true, force: true })
  }

  return path.join(outDir, platform === 'win32' ? 'rtk.exe' : 'rtk')
}

if (isMain(import.meta.url)) {
  const [a, b, c] = process.argv.slice(2)
  const platform = a || process.platform
  const arch = b || process.arch
  const outDir = c || path.resolve(import.meta.dirname, '..', 'build', 'factr')
  const out = await stageRtk({ platform, arch, outDir })
  console.log(`[stage-rtk] ${out} (${existsSync(out) ? 'ok' : 'missing'})`)
}
