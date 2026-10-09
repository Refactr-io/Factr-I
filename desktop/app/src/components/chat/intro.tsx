import { useState } from 'react'

export type IntroProps = {
  /** Kept for callers that still pass a personality; the greeting no longer depends on it. */
  personality?: string
  /** Re-rolls the greeting (a new chat passes a new seed). */
  seed?: number
}

type Moment = 'afternoon' | 'evening' | 'morning' | 'night' | 'weekday' | 'weekend'

// One short line on an empty chat. A phrase with a moment only shows then (a morning line in the morning, a
// weekend line on Saturday or Sunday); the rest fit any time.
const GREETINGS: readonly (readonly [string, Moment?])[] = [
  ['Coffee and curiosity', 'morning'],
  ['Slow morning, big ideas', 'morning'],
  ['Sunrise session', 'morning'],
  ['The kettle is on', 'morning'],
  ['Tea, toast and a stack trace', 'afternoon'],
  ['Windows down, tabs up', 'afternoon'],
  ['Evening, friend', 'evening'],
  ['Rain on the window, code on the screen', 'evening'],
  ['Warm mug, cold logs', 'evening'],
  ['Cozy corner, clear head', 'evening'],
  ['Moonlit chat', 'night'],
  ['Late-night thoughts welcome', 'night'],
  ['Quiet hours, loud ideas', 'night'],
  ['Starlight and semicolons', 'night'],
  ['Headphones on, ask me anything', 'night'],
  ['Weekend project?', 'weekend'],
  ['Another day, another diff', 'weekday'],
  ['Good to see you'],
  ['What are we making today?'],
  ['Fresh page, sharp pencil'],
  ['Back at it'],
  ["Let's untangle something"],
  ['Pour a cup, then ask away'],
  ["What's on your mind?"],
  ['Ready when you are'],
  ['A good day to ship something'],
  ['Bring the messy version'],
  ["Let's think it through"],
  ['Small steps, big builds'],
  ['Notebook open, mind open'],
  ['Half an idea is plenty'],
  ["Let's figure it out"],
  ['Somewhere between a plan and a hunch'],
  ['Show me the problem'],
  ['One thing at a time'],
  ['Hello again'],
  ['Where shall we begin?'],
  ['Make something good'],
  ['Deep breath, then deep work'],
  ['Think out loud with me']
]

/** The moments that hold at `date`: its part of the day, and weekend or weekday. */
export function momentsAt(date: Date): Moment[] {
  const hour = date.getHours()
  const day = date.getDay()
  const part: Moment = hour >= 5 && hour < 12 ? 'morning' : hour >= 12 && hour < 17 ? 'afternoon' : hour >= 17 && hour < 22 ? 'evening' : 'night'

  return [part, day === 0 || day === 6 ? 'weekend' : 'weekday']
}

/** Half the time a phrase for the current moment, otherwise one that fits any time. `seed` picks within the pool. */
export function pickGreeting(seed: number, date = new Date()): string {
  const now = momentsAt(date)
  const timed = GREETINGS.filter(([, moment]) => moment && now.includes(moment))
  const anytime = GREETINGS.filter(([, moment]) => !moment)
  const pool = timed.length > 0 && Math.abs(seed) % 2 === 0 ? timed : anytime

  return pool[Math.floor(Math.abs(seed) / 2) % pool.length]![0]
}

export function Intro({ seed }: IntroProps) {
  const [mountSeed] = useState(() => Math.floor(Math.random() * 100000))

  return (
    <div
      className="pointer-events-none flex w-full min-w-0 flex-col items-center justify-center px-0.5 py-6 text-center sm:px-6 lg:px-8"
      data-slot="aui_intro"
    >
      <h1 className="m-0 max-w-[26ch] text-balance text-[2rem] leading-[1.15] font-medium tracking-tight text-foreground/80">
        {pickGreeting(mountSeed + (seed ?? 0))}
      </h1>
    </div>
  )
}
