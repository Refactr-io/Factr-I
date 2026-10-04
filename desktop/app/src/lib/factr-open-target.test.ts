import { describe, expect, it } from 'vitest'

import {
  normalizeFactrOpenString,
  pathFromFactrDeepLink,
  pathFromOpenDeepLink,
  resolveFactrOpenPath
} from './factr-open-target'

describe('normalizeFactrOpenString', () => {
  it('accepts hash-router paths and strips a leading hash', () => {
    expect(normalizeFactrOpenString('/index-network/intent/1')).toBe('/index-network/intent/1')
    expect(normalizeFactrOpenString('#/index-network/intent/1')).toBe('/index-network/intent/1')
  })

  it('maps plugin-scoped factr:// deep links to the same path', () => {
    expect(normalizeFactrOpenString('factr://index-network/intent/1')).toBe('/index-network/intent/1')
    expect(normalizeFactrOpenString('factr://index-network/intent/1?focus=true')).toBe(
      '/index-network/intent/1?focus=true'
    )
  })

  it('maps factr://open/… deep links by stripping the open host', () => {
    expect(normalizeFactrOpenString('factr://open/index-network/intent/1')).toBe('/index-network/intent/1')
    expect(normalizeFactrOpenString('factr://open/settings/plugins')).toBe('/settings/plugins')
  })

  it('rejects reserved factr kinds and unsafe paths', () => {
    expect(normalizeFactrOpenString('factr://blueprint/morning-brief')).toBeNull()
    expect(normalizeFactrOpenString('factr://plugin/install')).toBeNull()
    expect(normalizeFactrOpenString('https://example.com/x')).toBeNull()
    expect(normalizeFactrOpenString('/../etc/passwd')).toBeNull()
    expect(normalizeFactrOpenString('index-network')).toBeNull()
  })
})

describe('resolveFactrOpenPath', () => {
  it('merges structured path + params', () => {
    expect(resolveFactrOpenPath({ path: '/index-network/intent/1', params: { focus: 'true' } })).toBe(
      '/index-network/intent/1?focus=true'
    )
  })

  it('resolves href the same as a bare string', () => {
    expect(resolveFactrOpenPath({ href: 'factr://index-network/intent/1' })).toBe('/index-network/intent/1')
  })
})

describe('pathFromFactrDeepLink', () => {
  it('builds the navigate path from a plugin-scoped deep-link payload', () => {
    expect(pathFromFactrDeepLink('index-network', 'intent/1')).toBe('/index-network/intent/1')
  })

  it('builds the navigate path from factr://open/… payloads', () => {
    expect(pathFromOpenDeepLink('index-network/intent/1')).toBe('/index-network/intent/1')
    expect(pathFromFactrDeepLink('open', 'agent/42')).toBe('/agent/42')
  })

  it('ignores reserved kinds', () => {
    expect(pathFromFactrDeepLink('blueprint', 'morning-brief')).toBeNull()
    expect(pathFromFactrDeepLink('plugin', 'install')).toBeNull()
  })
})
