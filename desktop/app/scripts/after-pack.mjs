/**
 * after-pack.mjs — electron-builder afterPack hook: flips the Electron fuses.
 *
 * A release build turns off the three fuses that let anyone run arbitrary code
 * inside the signed app process: RunAsNode (ELECTRON_RUN_AS_NODE=1 makes the
 * app binary a plain Node), EnableNodeOptionsEnvironmentVariable (NODE_OPTIONS
 * such as --require) and EnableNodeCliInspectArguments (--inspect and friends).
 * OnlyLoadAppFromAsar stays as Electron ships it: `asarUnpack` keeps dist/**
 * outside the archive and `main` is dist/electron-main.mjs.
 *
 * FACTR_QA_FUSES=1 builds the QA variant: the same three fuses explicitly ON
 * (Electron's defaults), so Playwright can drive it. Everything else about the
 * build is identical; only the fuse bytes differ.
 *
 * WHY a hook and not the `electronFuses` config key: the key is static in
 * package.json and the CLI's `-c.electronFuses.*=false` arrives as the string
 * "false" (truthy). The hook calls the same electron-builder API the key uses
 * (`packager.generateFuseConfig` + `packager.addElectronFuses`) and runs after
 * the bundle is assembled and before electron-builder signs it. A failure here
 * fails the build: a release must never ship with the fuses silently left on.
 */

import fs from 'node:fs'
import path from 'node:path'

export const QA_FUSES_ENV = 'FACTR_QA_FUSES'

/** Where the packed app keeps dist/ (asarUnpack: dist/**), per platform. */
function packedDistDir(context) {
  const name = context.packager?.appInfo?.productFilename ?? 'Factr-I'
  const resources =
    process.platform === 'darwin' ? path.join(context.appOutDir, `${name}.app`, 'Contents', 'Resources') : path.join(context.appOutDir, 'resources')

  return path.join(resources, 'app.asar.unpacked', 'dist')
}

/**
 * The bundle's own variant (dist/build-variant.json, written by bundle-electron-main.mjs) must match the variant
 * the fuses are being set for. The release variant strips Chromium's remote debugging switches and the QA variant
 * keeps them (electron/release-debug-lockdown.ts), so a stale dist/ from the other kind of build fails the build.
 */
export function assertBundleVariant(context, env = process.env) {
  const file = path.join(packedDistDir(context), 'build-variant.json')
  let qa
  try {
    qa = JSON.parse(fs.readFileSync(file, 'utf8')).qa
  } catch (error) {
    throw new Error(`after-pack: cannot read the bundle variant at ${file}: ${error.message}`)
  }
  const wantQa = env[QA_FUSES_ENV] === '1'
  if (qa !== wantQa) {
    throw new Error(
      `after-pack: the bundle is the ${qa ? 'QA' : 'release'} variant but this build is ${wantQa ? 'QA' : 'release'} (${QA_FUSES_ENV}=${env[QA_FUSES_ENV] ?? ''}); rebuild dist/ with the same environment`
    )
  }
}

/** The fuse options (electron-builder `electronFuses` shape) for this build. */
export function fuseOptionsFor(env, platform = process.platform, arch = process.arch) {
  const on = env[QA_FUSES_ENV] === '1'

  return {
    runAsNode: on,
    enableNodeOptionsEnvironmentVariable: on,
    enableNodeCliInspectArguments: on,
    // Flipping bytes breaks the arm64 ad-hoc signature; re-sign so the bundle still launches unsigned-by-identity.
    resetAdHocDarwinSignature: platform === 'darwin' && arch === 'arm64'
  }
}

export default async function afterPack(context) {
  assertBundleVariant(context)
  const options = fuseOptionsFor(process.env)
  const variant = options.runAsNode ? 'QA (fuses on)' : 'release (fuses off)'
  console.log(
    `[after-pack] ${variant}: RunAsNode=${options.runAsNode} NodeOptions=${options.enableNodeOptionsEnvironmentVariable} Inspect=${options.enableNodeCliInspectArguments}`
  )
  const config = await context.packager.generateFuseConfig(options)
  await context.packager.addElectronFuses(context, config)
}
