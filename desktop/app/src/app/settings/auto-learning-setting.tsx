import { useEffect, useState } from 'react'

import { useGatewayRequest } from '@/app/gateway/hooks/use-gateway-request'
import { useI18n } from '@/i18n'
import { notifyError } from '@/store/notifications'

import { ToggleRow } from './primitives'

/** factr-learn's auto-learning switch: the engine's `learning.enabled` setting, read and written over the config RPC. */
export function AutoLearningSetting() {
  const { t } = useI18n()
  const c = t.settings.config
  const { requestGateway } = useGatewayRequest()
  const [enabled, setEnabled] = useState<boolean | null>(null)
  const [supported, setSupported] = useState(true)

  useEffect(() => {
    let live = true

    requestGateway<{ value: unknown }>('config.get', { key: 'learning.enabled' })
      .then(r => live && setEnabled(r?.value !== false))
      .catch(err => {
        // Only the Factr-I engine has this setting; other backends say "unknown config key".
        if (/unknown config key/i.test(err instanceof Error ? err.message : String(err))) {
          live && setSupported(false)
        } else {
          notifyError(err, c.failedLoad)
        }
      })

    return () => {
      live = false
    }
  }, [c.failedLoad, requestGateway])

  const save = async (next: boolean) => {
    const previous = enabled
    setEnabled(next)

    try {
      await requestGateway('config.set', { key: 'learning.enabled', value: next })
    } catch (err) {
      setEnabled(previous)
      notifyError(err, c.autosaveFailed)
    }
  }

  if (!supported) {
    return null
  }

  return (
    <ToggleRow
      checked={enabled ?? false}
      description={c.autoLearningDesc}
      disabled={enabled === null}
      label={c.autoLearningTitle}
      onChange={next => void save(next)}
    />
  )
}
