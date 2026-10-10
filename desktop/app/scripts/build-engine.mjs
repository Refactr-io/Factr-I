// Release engine build with the build machine's paths remapped, so the binary does not embed
// /Users/<name>/.cargo/registry/... or the checkout / target directory in panic and debug strings.
//   node scripts/build-engine.mjs [extra cargo build args]
// rustc flags only: no source change, and CARGO_ENCODED_RUSTFLAGS keeps paths with spaces intact.
import { spawnSync } from 'node:child_process'
import os from 'node:os'
import path from 'node:path'
import { isMain } from './utils.mjs'

const repo = path.resolve(import.meta.dirname, '../../..')

/** `--remap-path-prefix` flags, most general first: rustc lets the last matching remap win. */
export function remapFlags({ home, cargoHome, repoRoot, targetDir }) {
  const pairs = [
    [home, '~'],
    [cargoHome, '/cargo'],
    [repoRoot, '/factr'],
    [targetDir, '/target']
  ]

  return pairs.filter(([from]) => from).map(([from, to]) => `--remap-path-prefix=${from}=${to}`)
}

/** The rustflags list cargo would use already (encoded form wins, like cargo), plus the remaps. */
export function encodedRustflags(env, remaps) {
  const existing = env.CARGO_ENCODED_RUSTFLAGS
    ? env.CARGO_ENCODED_RUSTFLAGS.split('\x1f')
    : (env.RUSTFLAGS || '').split(/\s+/)

  return [...existing.filter(Boolean), ...remaps].join('\x1f')
}

/**
 * CFLAGS for the C dependencies (aws-lc and kin compile their own __FILE__ paths): the same remaps as
 * `-ffile-prefix-map`. CFLAGS is split on whitespace, so a path with a space is left out.
 */
export function cflagsWithPrefixMaps(existing, remaps) {
  const maps = remaps
    .map(flag => flag.replace(/^--remap-path-prefix=/, ''))
    .filter(pair => !/\s/.test(pair))
    .map(pair => `-ffile-prefix-map=${pair}`)

  return [...(existing || '').split(/\s+/).filter(Boolean), ...maps].join(' ')
}

function main() {
  const env = { ...process.env }
  const home = os.homedir()
  const remaps = remapFlags({
    home,
    cargoHome: env.CARGO_HOME || path.join(home, '.cargo'),
    repoRoot: repo,
    targetDir: env.CARGO_TARGET_DIR ? path.resolve(env.CARGO_TARGET_DIR) : null
  })
  env.CARGO_ENCODED_RUSTFLAGS = encodedRustflags(env, remaps)
  env.CFLAGS = cflagsWithPrefixMaps(env.CFLAGS, remaps)
  delete env.RUSTFLAGS
  const args = ['build', '--release', '--manifest-path', path.join(repo, 'engine', 'Cargo.toml'), ...process.argv.slice(2)]
  const run = spawnSync('cargo', args, { stdio: 'inherit', env, shell: false })
  process.exit(run.status ?? 1)
}

if (isMain(import.meta.url)) main()
