// The app's only sound effects: four short cues in the manner of Claude's desktop app (mic on, mic off, approval
// needed, connection lost). Synthesized here from plain sine pings, so no audio asset ships. Everything else is silent.

import { getAudioContext } from '@/lib/audio-context'
import { $hapticsMuted } from '@/store/haptics'
import { $gatewayState } from '@/store/session'

interface Ping {
  /** Seconds after the cue starts. */
  at: number
  decay: number
  freq: number
  gain: number
}

// Pitches and timings follow the character of Claude's cues: a bright double tap going in, a lower triple echo going
// out, a rising two-note chime asking for approval, a soft low double thud for a dropped connection.
const CUES = {
  approval: [
    { at: 0, decay: 0.17, freq: 659, gain: 0.34 },
    { at: 0.15, decay: 0.36, freq: 880, gain: 0.4 }
  ],
  disconnected: [
    { at: 0.03, decay: 0.22, freq: 545, gain: 0.32 },
    { at: 0.15, decay: 0.38, freq: 541, gain: 0.26 }
  ],
  enterVoice: [
    { at: 0, decay: 0.14, freq: 968, gain: 0.42 },
    { at: 0.15, decay: 0.22, freq: 971, gain: 0.2 }
  ],
  exitVoice: [
    { at: 0, decay: 0.2, freq: 724, gain: 0.4 },
    { at: 0.1, decay: 0.24, freq: 724, gain: 0.28 },
    { at: 0.2, decay: 0.42, freq: 726, gain: 0.2 }
  ]
} as const satisfies Record<string, readonly Ping[]>

export type SfxCue = keyof typeof CUES

/** Play one cue, unless sounds are muted or the platform has no WebAudio. */
export function playSfx(cue: SfxCue): void {
  if ($hapticsMuted.get()) {
    return
  }

  const ac = getAudioContext()

  if (!ac) {
    return
  }

  const t0 = ac.currentTime + 0.005
  const master = ac.createGain()
  master.gain.value = 0.55
  master.connect(ac.destination)

  for (const ping of CUES[cue]) {
    const osc = ac.createOscillator()
    const env = ac.createGain()
    const start = t0 + ping.at

    osc.type = 'sine'
    osc.frequency.setValueAtTime(ping.freq, start)
    // Fast attack, exponential decay: a soft tap with no click at either end.
    env.gain.setValueAtTime(0.0001, start)
    env.gain.exponentialRampToValueAtTime(ping.gain, start + 0.006)
    env.gain.exponentialRampToValueAtTime(0.0001, start + ping.decay)
    osc.connect(env)
    env.connect(master)
    osc.start(start)
    osc.stop(start + ping.decay + 0.02)
  }
}

/** Sound once when an established gateway connection drops. Returns the unsubscribe. */
export function watchConnectionSfx(): () => void {
  let wasOpen = $gatewayState.get() === 'open'

  return $gatewayState.listen(state => {
    if (wasOpen && (state === 'closed' || state === 'error')) {
      playSfx('disconnected')
    }

    wasOpen = state === 'open'
  })
}
