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
export const GREETINGS: readonly (readonly [string, Moment?])[] = [
  ["Fresh light, fresh start.", 'morning'],
  ["Early hours, clear head.", 'morning'],
  ["Morning. Kettle on?", 'morning'],
  ["A quiet morning start.", 'morning'],
  ["Up with the sun?", 'morning'],
  ["Morning, welcome in.", 'morning'],
  ["First cup, first thought.", 'morning'],
  ["Slow morning, good company.", 'morning'],
  ["Afternoon, hello again.", 'afternoon'],
  ["A gentle afternoon.", 'afternoon'],
  ["Midday, still going?", 'afternoon'],
  ["Tea and afternoon light.", 'afternoon'],
  ["How's the afternoon?", 'afternoon'],
  ["Afternoon lull, welcome.", 'afternoon'],
  ["Evening light, easy pace.", 'evening'],
  ["Evening, welcome in.", 'evening'],
  ["Lamp on, evening in.", 'evening'],
  ["Evening, take a seat.", 'evening'],
  ["Winding down together?", 'evening'],
  ["A calm evening ahead.", 'evening'],
  ["Dusk settles in.", 'evening'],
  ["Late, and still curious.", 'night'],
  ["The quiet hours.", 'night'],
  ["Night owl, hello.", 'night'],
  ["Up late again?", 'night'],
  ["Starlight and a lamp.", 'night'],
  ["Midnight company.", 'night'],
  ["Quiet night, clear mind.", 'night'],
  ["The house is asleep.", 'night'],
  ["A slow weekend hello.", 'weekend'],
  ["Weekend, unhurried.", 'weekend'],
  ["No rush this weekend.", 'weekend'],
  ["Weekend tinkering?", 'weekend'],
  ["Weekend, welcome in.", 'weekend'],
  ["Another weekday, hello.", 'weekday'],
  ["Weekday rhythm, steady.", 'weekday'],
  ["A fresh workday.", 'weekday'],
  ["Back for the week?", 'weekday'],
  ["Hello, you."],
  ["Good to see you."],
  ["Pull up a chair."],
  ["What are you making?"],
  ["Hello again."],
  ["Quiet desk, open mind."],
  ["A blank page awaits."],
  ["Tea's warm."],
  ["Where were we?"],
  ["Ready when you are."],
  ["Small steps count."],
  ["Something new to build?"],
  ["Slow and steady."],
  ["Hi there, friend."],
  ["Room to think."],
  ["Curious about something?"],
  ["One thing at a time."],
  ["Glad you're here!"],
  ["Ideas welcome here."],
  ["Mug in hand?"],
  ["Clear head, warm cup."],
  ["What's next?"],
  ["Quiet focus."],
  ["A good place to start."],
  ["Thinking out loud?"],
  ["Picking up the thread?"],
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
