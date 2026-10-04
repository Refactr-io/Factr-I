import { useStore } from '@nanostores/react'
import { useEffect } from 'react'

import { takeGuideShape } from '@/components/onboarding-chat/assembly'
import { $introReveal } from '@/store/intro-reveal'
import { $onboardingGate, runGuideKickoff } from '@/store/onboarding-gate'

interface OnboardingChatGateProps {
  enabled: boolean
  onKickoff: () => Promise<boolean>
}

export function OnboardingChatGate({ enabled, onKickoff }: OnboardingChatGateProps) {
  const gate = useStore($onboardingGate)
  const intro = useStore($introReveal)

  // A guide is owed the moment the renderer knows it (cinematic with the film
  // seen, or a relaunch mid-guide). Take the solo shape now, before the
  // gateway opens. Otherwise the normal shell paints at full size for the
  // seconds the backend takes to come up, and then snaps down to the guide.
  useEffect(() => {
    if (gate.guideQueued && intro.phase === 'hidden') {
      takeGuideShape()
    }
    // Once, on mount: the queued flag is a boot fact, not a live signal.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  useEffect(() => {
    if (enabled && gate.guideQueued && intro.phase === 'hidden') {
      void runGuideKickoff(onKickoff)
    }
  }, [enabled, gate.guideQueued, intro.phase, onKickoff])

  return null
}
