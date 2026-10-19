import './connection-status.css'

import { useStore } from '@nanostores/react'
import { useEffect, useRef, useState } from 'react'

import { AgentOrb } from '@/components/ui/agent-orb'
import { Button } from '@/components/ui/button'
import { Loader } from '@/components/ui/loader'
import { useMediaQuery } from '@/hooks/use-media-query'
import { useI18n } from '@/i18n'
import { $desktopBoot } from '@/store/boot'
import { reconnectGateway } from '@/store/gateway-reconnect'
import { $gatewaySwitching } from '@/store/gateway-switch'
import { notifyError } from '@/store/notifications'
import { guidedOnboardingActive } from '@/store/onboarding-gate'
import { $gatewayState } from '@/store/session'

const EXIT_MS = 240

// Warm reloads connect too quickly to inspect. Keep the existing dev-only preview.
function forcedPreview(): boolean {
  return import.meta.env.DEV && new URLSearchParams(window.location.search).get('connecting') === '1'
}

export function GatewayConnectingOverlay() {
  const gateway = useStore($gatewayState)
  const boot = useStore($desktopBoot)
  const switching = useStore($gatewaySwitching)
  const { t } = useI18n()
  const reduce = useMediaQuery('(prefers-reduced-motion: reduce)')
  const [previewing] = useState(forcedPreview)
  const [leaving, setLeaving] = useState(false)
  const [gone, setGone] = useState(false)
  const [retrying, setRetrying] = useState(false)
  const shown = useRef(false)
  const completed = useRef(false)

  if (!boot.running && boot.progress >= 100 && !boot.error) {
    completed.current = true
  }

  const connecting = !completed.current && !switching && gateway !== 'open' && !boot.error &&
    (boot.visible || boot.running || boot.progress < 100)

  if (connecting || previewing) {
    shown.current = true
  }

  // Retarget immediately if the socket drops during the exit. Never strand a faded screen.
  useEffect(() => {
    if (connecting) {
      setLeaving(false)
      setGone(false)

      return
    }

    if (gateway !== 'open' || !shown.current || previewing) {
      return
    }

    setLeaving(true)
    const id = window.setTimeout(() => setGone(true), reduce ? 0 : EXIT_MS)

    return () => window.clearTimeout(id)
  }, [connecting, gateway, previewing, reduce])

  const retry = async () => {
    if (retrying) {
      return
    }

    setRetrying(true)
    try {
      await reconnectGateway()
    } catch (error) {
      notifyError(error, t.boot.errors.gatewayConnectionLost)
    } finally {
      setRetrying(false)
    }
  }

  if ((!previewing && guidedOnboardingActive()) || boot.error || switching) {
    return null
  }

  // A reconnect never covers an already usable chat or steals keyboard focus.
  if (completed.current && gateway !== 'open' && gateway !== 'idle') {
    return (
      <div className="connection-retry-notice" data-slot="connection-retry">
        <Loader label={t.boot.connection.reconnecting} />
        <Button disabled={retrying} onClick={() => void retry()} size="sm" variant="secondary">
          {t.boot.errors.reconnectNow}
        </Button>
      </div>
    )
  }

  if (!previewing && (gone || (!connecting && !shown.current) || (reduce && leaving))) {
    return null
  }

  const copy = t.boot.connection
  const status = boot.phase.includes('model') ? copy.loadingModel
    : boot.phase === 'renderer.config' ? t.boot.steps.loadingSettings
    : boot.phase.includes('remote') || boot.phase === 'renderer.gateway.connect' ? copy.connectingEngine
    : boot.phase.includes('runtime') || boot.phase.startsWith('bootstrap.') ? copy.preparingEngine
    : copy.startingEngine

  return (
    <div
      aria-label={status}
      className="connection-screen"
      data-glass-opaque=""
      data-leaving={leaving ? 'true' : undefined}
      data-slot="engine-connecting"
      role="status"
    >
      <div className="connection-screen__content">
        <AgentOrb paused={leaving} />
        <p className="connection-screen__label">{status}</p>
      </div>
    </div>
  )
}
