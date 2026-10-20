import { afterEach, beforeEach, describe, expect, it } from 'vitest'

import { fieldCopyForSchemaKey } from '@/app/settings/field-copy'

import { TRANSLATIONS } from './catalog'
import { setRuntimeI18nLocale, translateNow } from './runtime'
import type { Locale } from './types'
import { zh } from './zh'

describe('desktop i18n runtime translator', () => {
  beforeEach(() => {
    setRuntimeI18nLocale('en')
  })

  afterEach(() => {
    setRuntimeI18nLocale('en')
  })

  it('translates string paths for the active runtime locale', () => {
    setRuntimeI18nLocale('zh')

    expect(translateNow('boot.ready')).toBe(zh.boot.ready)
    expect(translateNow('assistant.tool.statusRecovered')).toBe(zh.assistant.tool.statusRecovered)
    expect(translateNow('boot.ready')).not.toBe(TRANSLATIONS.en.boot.ready)
  })

  it('passes arguments to function translations', () => {
    expect(translateNow('notifications.updateReadyMessage', 2)).toBe(
      TRANSLATIONS.en.notifications.updateReadyMessage(2)
    )
    expect(translateNow('notifications.updateReadyMessage', 2)).toContain('2')
  })

  it('keeps translated settings field copy addressable from schema keys', () => {
    const field = ['display', 'show_reasoning'].join('.')

    expect(fieldCopyForSchemaKey(zh.settings.fieldLabels, field)).toBeTypeOf('string')
    expect(fieldCopyForSchemaKey(zh.settings.fieldDescriptions, field)).toBeTypeOf('string')
  })

  it('falls back to English when the active locale cannot resolve a key', () => {
    const boot = TRANSLATIONS.ja.boot as { ready?: string }
    const originalReady = boot.ready

    try {
      boot.ready = undefined
      setRuntimeI18nLocale('ja')

      expect(translateNow('boot.ready')).toBe(TRANSLATIONS.en.boot.ready)
    } finally {
      boot.ready = originalReady
    }
  })

  it.each(Object.keys(TRANSLATIONS) as Locale[])('keeps obsolete cloud wording out of %s', locale => {
    const messages = JSON.stringify(TRANSLATIONS[locale], (_key, value) =>
      typeof value === 'function' ? value.toString() : value
    )

    expect(messages.includes('Factr-I Cloud')).toBe(false)
  })

  it.each(['zh', 'zh-hant', 'ja', 'ar', 'ru'] as const)(
    'uses current English gateway wording for removed overrides in %s',
    locale => {
      const keys = [
        'settings.connections.intro',
        'settings.connections.updateSkippedCloud',
        'settings.connections.kindCloud',
        'settings.connections.kindRemoteDesc',
        'settings.connections.kindCloudDesc',
        'settings.connections.cloudAddHint',
        'settings.gateway.remoteDesc',
        'settings.gateway.remoteAuthHint'
      ]

      if (locale !== 'ru') {
        keys.push('settings.gateway.intro')
      }

      for (const key of keys) {
        setRuntimeI18nLocale('en')
        const english = translateNow(key)
        setRuntimeI18nLocale(locale)

        expect(english).not.toBe(key)
        expect(translateNow(key), key).toBe(english)
      }

      expect(translateNow('settings.connections.cloudAddHint')).toBe('')
    }
  )

  it('returns the key when no locale can resolve a path', () => {
    setRuntimeI18nLocale('zh')

    expect(translateNow('missing.path')).toBe('missing.path')
  })
})
