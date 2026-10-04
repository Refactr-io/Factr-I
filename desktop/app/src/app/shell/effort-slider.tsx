import './effort-slider.css'

import { REASONING_EFFORTS, type ReasoningEffort } from '@factr/shared'
import { type CSSProperties, type KeyboardEvent, type PointerEvent, useState } from 'react'

import { Tip } from '@/components/ui/tooltip'
import { HelpCircle } from '@/lib/icons'

// The effort bar from React Bits' PromptBar (reactbits.dev), lifted on its own:
// the markup, geometry, easing and Max colouring are theirs; the ink and
// surfaces are mapped to Factr tokens in effort-slider.css.
const EDGE = 11

const stepAt = (i: number, last: number) => `calc(${EDGE}px + (100% - ${EDGE * 2}px) * ${i / Math.max(1, last)})`
const fillAt = (i: number, last: number) => (i === last ? '100%' : `calc(${stepAt(i, last)} + 7px)`)

/** The levels a model can actually serve, from its catalog `efforts[]`, in ladder order.
 *  `undefined` (the catalog does not say) keeps the full ladder. */
export function supportedEfforts(raw: unknown): readonly ReasoningEffort[] | undefined {
  if (!Array.isArray(raw)) {
    return undefined
  }

  const wanted = new Set(raw.map(level => String(level).trim().toLowerCase()))
  const levels = REASONING_EFFORTS.filter(level => wanted.has(level))

  return levels.length ? levels : undefined
}

/** Index of `value` in `levels`; a level the model lacks snaps to the nearest one below it. */
function levelIndex(levels: readonly ReasoningEffort[], value: string): number {
  const exact = levels.indexOf(value as ReasoningEffort)

  if (exact >= 0) {
    return exact
  }

  const rank = REASONING_EFFORTS.indexOf(value as ReasoningEffort)
  let best = 0

  levels.forEach((level, i) => {
    if (REASONING_EFFORTS.indexOf(level) <= rank) {
      best = i
    }
  })

  return best
}

export const isMaxEffort = (level: string) => level === 'max' || level === 'ultra'

interface EffortSliderProps {
  /** Translated copy: `effort`, `effortHelp`, `faster`, `smarter` and a name
   *  per level. */
  copy: Record<ReasoningEffort | 'effort' | 'effortHelp' | 'faster' | 'smarter', string>
  /** Small line under the track (the route-clamp note), if any. */
  note?: string
  /** Commit a level: on release of a drag, and on each keyboard step. */
  onCommit: (level: ReasoningEffort) => void
  /** Levels the model serves (catalog `efforts[]`); omitted = the full ladder. */
  levels?: readonly ReasoningEffort[]
  /** The committed level. */
  value: string
}

export function EffortSlider({ copy, levels = REASONING_EFFORTS, note, onCommit, value }: EffortSliderProps) {
  const LAST = levels.length - 1
  const committed = levelIndex(levels, value)

  // A drag previews locally and writes once, on release: every commit is a
  // session write. Dropped whenever the committed level moves, so an
  // optimistic write or a rollback always wins over a stale local pick.
  const [preview, setPreview] = useState<{ for: number; index: number } | null>(null)
  const index = preview && preview.for === committed ? preview.index : committed
  const level = levels[index]

  const clamp = (i: number) => Math.max(0, Math.min(LAST, i))

  const fromPointer = (e: PointerEvent<HTMLDivElement>) => {
    const rect = e.currentTarget.getBoundingClientRect()
    const k = (e.clientX - rect.left - EDGE) / Math.max(1, rect.width - 2 * EDGE)

    return clamp(Math.round(k * LAST))
  }

  // The picked level stays shown until the owner's value catches up (the
  // preview is keyed to the committed level it was made against).
  const commit = (i: number) => {
    setPreview({ for: committed, index: i })

    if (i !== committed) {
      onCommit(levels[i])
    }
  }

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const step =
      e.key === 'ArrowRight' || e.key === 'ArrowUp' ? 1 : e.key === 'ArrowLeft' || e.key === 'ArrowDown' ? -1 : 0

    const next = step ? clamp(index + step) : e.key === 'Home' ? 0 : e.key === 'End' ? LAST : null

    if (next === null) {
      return
    }

    // Keep the key from the owning menu (it would move focus or close a sub).
    e.preventDefault()
    e.stopPropagation()
    commit(next)
  }

  return (
    <div className="effort-bar" data-max={isMaxEffort(level) ? '' : undefined}>
      <div className="effort-bar__head">
        <span className="effort-bar__title">{copy.effort}</span>
        <span className="effort-bar__level">{copy[level]}</span>
        <Tip label={copy.effortHelp} side="top">
          <span aria-label={copy.effortHelp} className="effort-bar__help" role="img">
            <HelpCircle className="size-3.5" />
          </span>
        </Tip>
      </div>
      <div className="effort-bar__ends">
        <span>{copy.faster}</span>
        <span>{copy.smarter}</span>
      </div>
      <div
        aria-label={copy.effort}
        aria-valuemax={LAST}
        aria-valuemin={0}
        aria-valuenow={index}
        aria-valuetext={copy[level]}
        className="effort-bar__track"
        onKeyDown={onKeyDown}
        onPointerCancel={() => setPreview(null)}
        onPointerDown={e => {
          if (e.button !== 0) {
            return
          }

          try {
            e.currentTarget.setPointerCapture(e.pointerId)
          } catch {
            // Capture is a nicety (drags that leave the track keep tracking).
          }

          e.currentTarget.focus({ preventScroll: true })
          setPreview({ for: committed, index: fromPointer(e) })
        }}
        onPointerMove={e => {
          if (e.buttons & 1) {
            setPreview({ for: committed, index: fromPointer(e) })
          }
        }}
        onPointerUp={e => commit(fromPointer(e))}
        role="slider"
        style={{ '--effort-x': stepAt(index, LAST), '--effort-fill': fillAt(index, LAST) } as CSSProperties}
        tabIndex={0}
      >
        <span className="effort-bar__fill" />
        {levels.map((stop, i) => (
          <i className="effort-bar__dot" key={stop} style={{ left: stepAt(i, LAST) }} />
        ))}
        <span className="effort-bar__thumb" />
      </div>
      {note ? <div className="effort-bar__note">{note}</div> : null}
    </div>
  )
}
