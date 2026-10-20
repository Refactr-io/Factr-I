import { useStore } from '@nanostores/react'
import type { ReactNode } from 'react'

import { Button } from '@/components/ui/button'
import { useI18n } from '@/i18n'
import { triggerHaptic } from '@/lib/haptics'
import { Bell } from '@/lib/icons'
import { cn } from '@/lib/utils'
import { $hapticsMuted, setHapticsMuted } from '@/store/haptics'
import {
  $nativeNotifyPrefs,
  NATIVE_NOTIFICATION_KINDS,
  sendTestNativeNotification,
  setNativeNotifyEnabled,
  setNativeNotifyKind
} from '@/store/native-notifications'
import { notify } from '@/store/notifications'

import { SectionHeading, SettingsContent, ToggleRow } from './primitives'

const CAPTION = 'text-[length:var(--conversation-caption-font-size)] text-(--ui-text-tertiary)'

function Caption({ children, className }: { children: ReactNode; className?: string }) {
  return <p className={cn(CAPTION, className)}>{children}</p>
}

interface NotificationsSettingsProps {
  subpage?: string
}

export function NotificationsSettings({ subpage }: NotificationsSettingsProps = {}) {
  const { t } = useI18n()
  const prefs = useStore($nativeNotifyPrefs)
  const soundsMuted = useStore($hapticsMuted)
  const copy = t.settings.notifications
  const showAlerts = subpage === undefined || subpage === 'alerts'
  const showSounds = subpage === undefined || subpage === 'sounds'

  const runTest = async () => {
    triggerHaptic('open')
    const ok = await sendTestNativeNotification(copy.testTitle, copy.testBody)
    notify({ kind: ok ? 'info' : 'error', message: ok ? copy.testSent : copy.testUnsupported })
  }

  return (
    <SettingsContent>
      {showAlerts && (
        <>
          {subpage === undefined && <SectionHeading icon={Bell} title={copy.title} />}
          <Caption className="mb-2 leading-(--conversation-caption-line-height)">{copy.intro}</Caption>

          <ToggleRow
            checked={prefs.enabled}
            description={copy.enableAllDesc}
            label={copy.enableAll}
            onChange={setNativeNotifyEnabled}
          />

          {NATIVE_NOTIFICATION_KINDS.map(kind => (
            <ToggleRow
              checked={prefs.enabled && prefs.kinds[kind]}
              description={copy.kinds[kind].description}
              disabled={!prefs.enabled}
              key={kind}
              label={copy.kinds[kind].label}
              onChange={on => setNativeNotifyKind(kind, on)}
            />
          ))}
        </>
      )}

      {showSounds && (
        <ToggleRow
          checked={!soundsMuted}
          description={copy.soundEffectsDesc}
          label={copy.soundEffectsTitle}
          onChange={on => setHapticsMuted(!on)}
        />
      )}

      {showAlerts && (
        <div className="mt-4 flex flex-col gap-2">
          <Button className="self-start" onClick={() => void runTest()} size="sm" type="button" variant="outline">
            <Bell />
            {copy.test}
          </Button>
          <Caption>{copy.focusedHint}</Caption>
        </div>
      )}
    </SettingsContent>
  )
}
