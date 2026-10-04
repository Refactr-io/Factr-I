import { type ReasoningEffort } from '@factr/shared'
import { AnimatePresence, motion, useReducedMotion } from 'motion/react'

import { DropdownMenuItem, DropdownMenuSubContent } from '@/components/ui/dropdown-menu'
import { Switch } from '@/components/ui/switch'
import { useI18n } from '@/i18n'
import { isThinkingEnabled, reasoningEffortClamp, resolveReasoningEffort } from '@/lib/reasoning-effort'
import { cn } from '@/lib/utils'

import { EffortSlider } from './effort-slider'

// Factr' real reasoning levels live in lib/reasoning-effort; `none` is owned
// by the Thinking toggle, not the effort slider.

/** How "fast" is achieved for a given model — two different mechanisms:
 *  - `param`: the Anthropic/OpenAI `speed=fast` request parameter.
 *  - `variant`: a separate `…-fast` sibling model selected via the model field.
 */
export type FastControl =
  { kind: 'none' } | { kind: 'param'; on: boolean } | { kind: 'variant'; baseId: string; fastId: string; on: boolean }

/** Resolve the fast mechanism for a model: prefer the speed=fast parameter
 *  when the backend supports it, else fall back to a `…-fast` sibling model. */
export function resolveFastControl(
  model: string,
  providerModels: readonly string[],
  paramSupported: boolean,
  currentFastMode: boolean
): FastControl {
  if (paramSupported) {
    return { kind: 'param', on: currentFastMode }
  }

  if (/-fast$/i.test(model)) {
    const baseId = model.replace(/-fast$/i, '')

    // Only a toggle if there's a base to switch back to; otherwise it's a
    // standalone fast model with no "off" state.
    return providerModels.includes(baseId) ? { kind: 'variant', baseId, fastId: model, on: true } : { kind: 'none' }
  }

  const fastId = `${model}-fast`

  if (providerModels.includes(fastId)) {
    return { kind: 'variant', baseId: model, fastId, on: false }
  }

  // Fast isn't natively offered here, but if the session still has the speed
  // param on (carried over from a previous model), expose the toggle so it can
  // be turned off rather than stranded.
  if (currentFastMode) {
    return { kind: 'param', on: true }
  }

  return { kind: 'none' }
}

interface ModelEditSubmenuProps {
  /** Whether this model can turn thinking off. False on reasoning-mandatory
   *  routes, whose upstream rejects a disable — the toggle is hidden rather
   *  than offered as a control that silently does nothing. */
  canDisableReasoning?: boolean
  /** The profile's configured default effort — what an unset row inherits.
   *  Passed in (not read from a store) so this submenu stays pure. */
  defaultEffort: string
  /** This row's effective reasoning effort (live for the active model, else its
   *  preset) — the submenu shows and edits from this, never the raw session. */
  effort: string
  /** Gateway-reported level the route actually sends for `effort` (active row
   *  only; '' = unknown). A clamped pick is spelled out under the effort slider. */
  effortWire?: string
  /** How fast mode is offered for this model (param toggle vs. variant swap). */
  fastControl: FastControl
  /** Whether this row's model is the active one. */
  isActive: boolean
  /** This row's model id. */
  model: string
  /** Switch to a specific model id (used to swap base ⇄ -fast variant). */
  onSelectModel: (model: string) => Promise<boolean | void> | void
  /** Report an option change. This submenu is PURE: it never writes to a
   *  session, a preset store, or the gateway itself — the owning surface's
   *  controller decides what an edit means. That's what lets the same submenu
   *  drive a live chat session and a detached per-task override. */
  onSetOptions: (patch: { effort?: string; fast?: boolean }) => void
  /** This row's provider slug. */
  provider: string
  /** Whether this model supports reasoning effort. */
  reasoning: boolean
  /** Effort levels the catalog says this model serves; omitted = every level. */
  efforts?: readonly ReasoningEffort[]
}

export function ModelEditSubmenu(props: ModelEditSubmenuProps) {
  // The panel mounts one of these per model row; only the hovered row's
  // submenu is ever open. Keep this wrapper hook-free and render the body as
  // a CHILD of SubContent so Radix's Presence gate leaves it unrendered until
  // the sub actually opens — eagerly running the body's hooks/JSX for every
  // row made opening the menu itself lag on large catalogs.
  return (
    <DropdownMenuSubContent className="w-52 p-0" sideOffset={4}>
      <ModelOptionsContent {...props} />
    </DropdownMenuSubContent>
  )
}

/** The options rows themselves, container-free: the catalog mounts them in a
 *  per-row submenu, the composer's reasoning pill in its own top-level menu. */
export function ModelOptionsContent({
  canDisableReasoning,
  defaultEffort,
  effort,
  effortWire,
  efforts,
  fastControl,
  isActive,
  onSelectModel,
  onSetOptions,
  reasoning
}: ModelEditSubmenuProps) {
  const { t } = useI18n()
  const copy = t.shell.modelOptions

  const effortValue = resolveReasoningEffort(effort, defaultEffort)
  const clamp = reasoningEffortClamp(effortValue, effortWire)
  const thinkingOn = isThinkingEnabled(effort, defaultEffort)
  const showThinkingToggle = reasoning && canDisableReasoning !== false

  const setFast = (enabled: boolean) => {
    if (fastControl.kind === 'variant') {
      // Fast is a separate model id. Report the choice so the controller can
      // record it against the base model, and only swap models now if this is
      // the active row — inactive edits stay preference-only.
      onSetOptions({ fast: enabled })

      if (isActive) {
        void onSelectModel(enabled ? fastControl.fastId : fastControl.baseId)
      }

      return
    }

    if (fastControl.kind === 'param') {
      onSetOptions({ fast: enabled })
    }
  }

  const hasFast = fastControl.kind !== 'none'
  const fastOn = fastControl.kind === 'none' ? false : fastControl.on

  return !hasFast && !reasoning ? (
    <div className="px-2.5 py-3 text-xs text-(--ui-text-tertiary)">{copy.noOptions}</div>
  ) : (
    // One panel, one inset: every row starts on the same 14px edge and the
    // sections are grouped by space, not rules.
    <div className="effort-panel">
      {showThinkingToggle ? (
        <PanelSwitchRow
          checked={thinkingOn}
          label={copy.thinking}
          onCheckedChange={checked => onSetOptions({ effort: checked ? effortValue || defaultEffort : 'none' })}
        />
      ) : null}
      {reasoning ? (
        <AnimatePresence initial={false} mode="popLayout">
          {thinkingOn ? (
            <PanelSection key="effort">
              <EffortSlider
                copy={copy}
                levels={efforts}
                note={clamp ? copy.sendsOnRoute(copy[clamp.wire]) : undefined}
                onCommit={level => onSetOptions({ effort: level })}
                value={effortValue}
              />
            </PanelSection>
          ) : showThinkingToggle ? (
            // Off is a state, not an empty popover: say what it means.
            <PanelSection key="off">
              <p className="effort-panel__hint">{copy.thinkingOffHint}</p>
            </PanelSection>
          ) : null}
        </AnimatePresence>
      ) : null}
      {hasFast ? <PanelSwitchRow checked={fastOn} label={copy.fast} onCheckedChange={setFast} spaced /> : null}
    </div>
  )
}

/** A label and a switch on the panel's edge. Still a menu item, so arrow keys
 *  reach it and Enter/Space toggle it; its highlight overhangs the inset. */
function PanelSwitchRow({
  checked,
  label,
  onCheckedChange,
  spaced = false
}: {
  checked: boolean
  label: string
  onCheckedChange: (checked: boolean) => void
  spaced?: boolean
}) {
  return (
    <DropdownMenuItem
      className={cn('effort-panel__row', spaced && 'mt-3')}
      onSelect={event => {
        event.preventDefault()
        onCheckedChange(!checked)
      }}
    >
      <span className="effort-panel__label">{label}</span>
      <Switch
        checked={checked}
        className="effort-panel__switch ml-auto"
        onCheckedChange={onCheckedChange}
        onClick={event => event.stopPropagation()}
        size="xs"
      />
    </DropdownMenuItem>
  )
}

/** A section that grows in and collapses out, so toggling thinking resizes
 *  the popover instead of snapping it. */
function PanelSection({ children }: { children: React.ReactNode }) {
  const reduce = useReducedMotion()

  return (
    <motion.div
      animate={{ height: 'auto', opacity: 1 }}
      className="overflow-hidden"
      exit={{ height: 0, opacity: 0 }}
      initial={{ height: 0, opacity: 0 }}
      transition={reduce ? { duration: 0 } : { duration: 0.2, ease: [0.23, 1, 0.32, 1] }}
    >
      {children}
    </motion.div>
  )
}
