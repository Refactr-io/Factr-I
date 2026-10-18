// Scrollbars are overlays that show only while their area is being scrolled (or hovered) and fade out on their own.
// macOS draws permanent legacy bars when a mouse is attached or "Always show scroll bars" is on, and Chromium does not
// animate ::-webkit-scrollbar-thumb itself, so the show/hide is ours: one capture-phase listener tweens a `--sb-a`
// (0..1) custom property on whichever element just scrolled; styles.css multiplies the thumb colour by it. Nothing is
// attached per element, and only the element that scrolls is ever touched.

const IDLE_MS = 800
const IN_MS = 180
const OUT_MS = 300

interface State {
  raf: number
  timer: ReturnType<typeof setTimeout> | undefined
  value: number
}

const states = new WeakMap<Element, State>()

const easeOut = (t: number) => 1 - (1 - t) ** 3

function tween(el: HTMLElement, state: State, to: number, ms: number, done?: () => void) {
  cancelAnimationFrame(state.raf)

  const from = state.value
  const start = performance.now()

  if (ms === 0 || from === to) {
    state.value = to
    el.style.setProperty('--sb-a', String(to))
    done?.()

    return
  }

  const step = (now: number) => {
    const t = Math.min(1, (now - start) / ms)

    state.value = from + (to - from) * easeOut(t)
    el.style.setProperty('--sb-a', state.value.toFixed(3))

    if (t < 1) {
      state.raf = requestAnimationFrame(step)
    } else {
      done?.()
    }
  }

  state.raf = requestAnimationFrame(step)
}

export function installScrollActivity(doc: Document = document): () => void {
  const reduce = doc.defaultView?.matchMedia?.('(prefers-reduced-motion: reduce)')

  const onScroll = (event: Event) => {
    const target = event.target
    const el = target instanceof HTMLElement ? target : target === doc ? doc.documentElement : null

    if (!el) {
      return
    }

    let state = states.get(el)

    if (!state) {
      state = { raf: 0, timer: undefined, value: 0 }
      states.set(el, state)
    }

    const s = state

    clearTimeout(s.timer)

    if (s.value < 1) {
      tween(el, s, 1, reduce?.matches ? 0 : IN_MS)
    }

    s.timer = setTimeout(
      () => tween(el, s, 0, reduce?.matches ? 0 : OUT_MS, () => el.style.removeProperty('--sb-a')),
      IDLE_MS
    )
  }

  doc.addEventListener('scroll', onScroll, { capture: true, passive: true })

  return () => doc.removeEventListener('scroll', onScroll, { capture: true })
}
