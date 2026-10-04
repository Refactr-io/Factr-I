import { useStore } from '@nanostores/react'
import type { OrbState } from 'thinking-orbs'

import { AgentOrb } from '@/components/ui/agent-orb'
import { Button } from '@/components/ui/button'
import { Codicon } from '@/components/ui/codicon'
import { Tip, TipKeybindLabel } from '@/components/ui/tooltip'
import { useI18n } from '@/i18n'
import { triggerHaptic } from '@/lib/haptics'
import { CornerDownLeft, Ear, EarOff, iconSize, Square } from '@/lib/icons'
import { cn } from '@/lib/utils'
import { $hudMode, closeHud, resetHudLayout } from '@/store/hud'
import { $wakeWord, toggleWakeWord } from '@/store/wake-word'

import { ACTIVE_ICON_BTN, GHOST_ICON_BTN, PRIMARY_ICON_BTN, RETURN_ICON_BTN } from './control-classes'
import type { ConversationStatus } from './hooks/use-voice-conversation'
import { ModelPill } from './model-pill'
import { ReasoningPill } from './reasoning-pill'
import { StartVoiceButton } from './start-voice-button'
import type { ChatBarState, VoiceStatus } from './types'
import { DictateButton, VoiceMenu } from './voice-menu'

// Re-exported: `context-menu.tsx` and other row neighbours have always reached
// for these here, and the row is where they read as belonging.
export { ACTIVE_ICON_BTN, GHOST_ICON_BTN, ICON_BTN, PRIMARY_ICON_BTN } from './control-classes'

interface ConversationProps {
  active: boolean
  level: number
  muted: boolean
  status: ConversationStatus
  onEnd: () => void
  onStart: () => void
  onStopTurn: () => void
  onToggleMute: () => void
}

export function ComposerControls({
  autoSpeak,
  busy,
  busyAction,
  canSubmit,
  compactModelPill = false,
  conversation,
  disabled,
  foldVoice = false,
  hasComposerPayload,
  hideModelPill = false,
  minimal = false,
  part = 'all',
  state,
  voiceStatus,
  onDictate,
  onToggleAutoSpeak
}: {
  autoSpeak: boolean
  busy: boolean
  busyAction: 'steer' | 'queue' | 'stop'
  canSubmit: boolean
  compactModelPill?: boolean
  conversation: ConversationProps
  disabled: boolean
  foldVoice?: boolean
  hasComposerPayload: boolean
  hideModelPill?: boolean
  minimal?: boolean
  /** Claude Code layout: `box` is what sits inside the input box (queue, the
   *  return/stop glyph), `row` the controls row under it (model, effort, the
   *  call mic). `all` is the single-row composer (HUD, popped out). */
  part?: 'all' | 'box' | 'row' | 'voice'
  state: ChatBarState
  voiceStatus: VoiceStatus
  onDictate: () => void
  onToggleAutoSpeak: () => void
}) {
  const { t } = useI18n()
  const c = t.composer
  const hudMode = useStore($hudMode)

  if (conversation.active) {
    return part === 'row' ? null : <ConversationPill {...conversation} disabled={disabled} />
  }

  // Claude Code's mic + chevron: the mic dictates, the chevron opens every
  // other voice control as a menu (no fan opening over the input). One split
  // control: hovering either half (or an open menu) outlines the pair, and the
  // half under the pointer takes the fill.
  const voicePair = (
    <span
      className={cn(
        'flex items-center rounded-md border border-transparent transition-colors duration-150',
        'hover:border-foreground/15 has-[[data-state=open]]:border-foreground/15',
        '[&>*:first-child]:rounded-r-none [&>*:last-child]:rounded-l-none',
        '[&>*:last-child[data-state=open]]:bg-(--chrome-action-hover)'
      )}
      data-voice-pair=""
    >
      <DictateButton disabled={disabled} onDictate={onDictate} state={state} voiceStatus={voiceStatus} />
      <VoiceMenu
        autoSpeak={autoSpeak}
        chevronTrigger
        disabled={disabled}
        onDictate={onDictate}
        onStartConversation={conversation.onStart}
        onToggleAutoSpeak={onToggleAutoSpeak}
        state={state}
        voiceStatus={voiceStatus}
      />
    </span>
  )

  if (part === 'voice') {
    return voicePair
  }

  if (part === 'row') {
    return (
      <div className="flex min-w-0 shrink items-center gap-(--composer-control-gap)">
        {hideModelPill ? null : (
          <>
            {/* The row has its own line, so the model name truncates to fit
                rather than folding to a bare chevron; effort (one short word)
                always stays - folding it away left a narrow pane no way to
                change it. */}
            <ModelPill disabled={disabled} model={state.model} />
            <ReasoningPill disabled={disabled} model={state.model} />
          </>
        )}
      </div>
    )
  }

  if (part === 'box') {
    const stop = busy && !hasComposerPayload
    // Mid-turn the one button does what Enter does (Settings: queue or send
    // now), so it is named for that; there is no separate queue control.
    const label = stop ? c.stop : busy && busyAction === 'queue' ? c.queueMessage : c.send

    // Nothing typed and nothing running: the box offers a voice call (the
    // white waveform disc, Codex's); typing turns it into the return glyph.
    if (!busy && !hasComposerPayload) {
      return <StartVoiceButton disabled={disabled} label={c.startVoice} onStart={conversation.onStart} />
    }

    return (
      <Tip label={<TipKeybindLabel actionId="composer.send" text={label} />} placement="control">
        <Button
          aria-label={label}
          className={RETURN_ICON_BTN}
          disabled={disabled || (!stop && !canSubmit)}
          type="submit"
          variant="ghost"
        >
          {stop ? (
            <span className="block size-2.5 rounded-sm bg-current" />
          ) : (
            <CornerDownLeft className={iconSize.md} />
          )}
        </Button>
      </Tip>
    )
  }

  const showVoicePrimary = !busy && !hasComposerPayload
  // Steer is just send: a payload keeps the Send affordance mid-turn. Stop
  // only when the composer is empty and a turn is running.
  const showStop = busy && !hasComposerPayload
  // The HUD is a Spotlight bar a few hundred pixels wide, so the four separate
  // voice toggles fold into one menu there and leave the row to the input. A
  // narrow tile hits the same wall from the other direction and folds for the
  // same reason — same controls, same state, different budget. Below that
  // even the menu goes: at `minimal` the row is the send button and nothing
  // else, which is the one thing that must survive every width.
  const foldedVoice = hudMode || foldVoice

  const voiceControls = foldedVoice ? (
    <VoiceMenu
      autoSpeak={autoSpeak}
      disabled={disabled}
      onDictate={onDictate}
      onStartConversation={conversation.onStart}
      onToggleAutoSpeak={onToggleAutoSpeak}
      state={state}
      voiceStatus={voiceStatus}
    />
  ) : (
    voicePair
  )

  return (
    <div className="flex min-w-0 shrink items-center gap-(--composer-control-gap)">
      {minimal ? null : (
        <>
          {hideModelPill ? null : (
            <>
              <ModelPill compact={compactModelPill} disabled={disabled} model={state.model} />
              {compactModelPill ? null : <ReasoningPill disabled={disabled} model={state.model} />}
            </>
          )}
          {voiceControls}
        </>
      )}
      {showVoicePrimary ? (
        <StartVoiceButton disabled={disabled} label={c.startVoice} onStart={conversation.onStart} />
      ) : (
        <Tip
          label={
            showStop ? (
              <TipKeybindLabel actionId="composer.send" text={c.stop} />
            ) : (
              <TipKeybindLabel actionId="composer.send" text={c.send} />
            )
          }
          placement="control"
        >
          <Button
            aria-label={showStop ? c.stop : c.send}
            className={PRIMARY_ICON_BTN}
            disabled={disabled || !canSubmit}
            type="submit"
          >
            {showStop ? (
              <span className="block size-2.5 rounded-sm bg-current" />
            ) : (
              <Codicon name="arrow-up" size="0.875rem" />
            )}
          </Button>
        </Tip>
      )}
      {/* The way out of HUD mode, riding the controls row rather than floating
          above the bar. The old chip lived in a 26px transparent strip reserved
          over the composer (--hud-chip-strip), which under glass is bare
          untinted material with a hidden button in it — a band of chrome above
          the surface, paid for in every state, for a control that is invisible
          until hovered. Here it costs no reserved space and sits with the other
          things you can press. */}
      {hudMode ? <HudWindowButtons /> : null}
    </div>
  )
}

function HudWindowButtons() {
  const { t } = useI18n()

  return (
    <>
      <Tip label={t.titlebar.resetHudLayout} placement="toolbar">
        <Button
          aria-label={t.titlebar.resetHudLayout}
          className={cn(GHOST_ICON_BTN, 'p-0')}
          onClick={resetHudLayout}
          size="icon"
          type="button"
          variant="ghost"
        >
          <Codicon name="discard" size="0.875rem" />
        </Button>
      </Tip>
      <Tip label={t.titlebar.exitHud} placement="toolbar">
        <Button
          aria-label={t.titlebar.exitHud}
          className={cn(GHOST_ICON_BTN, 'p-0')}
          onClick={closeHud}
          size="icon"
          type="button"
          variant="ghost"
        >
          <Codicon name="screen-normal" size="0.875rem" />
        </Button>
      </Tip>
    </>
  )
}

function ConversationPill({
  disabled,
  muted,
  onEnd,
  onStopTurn,
  onToggleMute,
  status
}: ConversationProps & { disabled: boolean }) {
  const { t } = useI18n()
  const c = t.composer
  const speaking = status === 'speaking'
  const listening = status === 'listening' && !muted

  const label =
    status === 'speaking'
      ? c.speaking
      : status === 'transcribing'
        ? c.transcribing
        : status === 'thinking'
          ? c.thinking
          : muted
            ? c.muted
            : c.listening

  // One orb reads the call's state; muted shows the struck mic instead.
  const orbState: OrbState | null = muted
    ? null
    : speaking
      ? 'breathing'
      : status === 'thinking' || status === 'transcribing'
        ? 'solving'
        : 'listening'

  return (
    <div className="ml-auto flex shrink-0 items-center gap-(--composer-control-gap)">
      {orbState ? <AgentOrb state={orbState} /> : null}
      {/* Keep the ear visible during voice chat — shown paused, since the
          conversation holds the mic (the one time wake must not listen). */}
      <WakeWordButton disabled={disabled} pausedForVoice />
      <Tip label={muted ? c.unmuteMic : c.muteMic} placement="control">
        <Button
          aria-label={muted ? c.unmuteMic : c.muteMic}
          aria-pressed={muted}
          className={cn(GHOST_ICON_BTN, 'p-0', muted && 'bg-muted text-muted-foreground')}
          disabled={disabled}
          onClick={() => {
            triggerHaptic('selection')
            onToggleMute()
          }}
          size="icon"
          type="button"
          variant="ghost"
        >
          <Codicon name={muted ? 'mic-off' : 'mic'} size="1rem" />
        </Button>
      </Tip>
      {listening && (
        <Button
          aria-label={c.stopListening}
          className="h-(--composer-control-size) shrink-0 gap-1.5 rounded-md px-2.5 text-xs text-muted-foreground hover:bg-accent hover:text-foreground"
          disabled={disabled}
          onClick={() => {
            triggerHaptic('submit')
            onStopTurn()
          }}
          type="button"
          variant="ghost"
        >
          <Square className={cn('fill-current', iconSize.xs)} />
          <span>{c.stopShort}</span>
        </Button>
      )}
      <Button
        aria-label={c.endConversation}
        className="h-(--composer-control-size) gap-1.5 rounded-md bg-primary px-3 text-xs font-medium text-primary-foreground hover:bg-primary/90"
        disabled={disabled}
        onClick={() => {
          triggerHaptic('close')
          onEnd()
        }}
        type="button"
      >
        <span>{c.endShort}</span>
      </Button>
      <span className="sr-only" role="status">
        {label}
      </span>
    </div>
  )
}

// "Hey Factr" wake-word toggle. ALWAYS rendered — the ear never hides. A
// user must always be able to click it to turn passive listening on; if the
// backend can't start (missing STT/TTS, deps still installing, no mic
// permission, etc.) the click surfaces the reason in the tooltip and the
// toggle stays off. States: listening (accent-highlighted), off (muted
// ear-off), and paused-for-voice (disabled while a voice conversation holds
// the mic — the one time wake genuinely must not listen). Backend refusals
// ({started:false, reason}) keep the toggle off and put the reason/hint in
// the tooltip.
function WakeWordButton({ disabled, pausedForVoice = false }: { disabled: boolean; pausedForVoice?: boolean }) {
  const { t } = useI18n()
  const c = t.composer
  const wake = useStore($wakeWord)

  const phrase = wake.phrase || 'hey factr'

  const label = pausedForVoice
    ? c.wakeWordPausedVoice(phrase)
    : wake.listening
      ? c.wakeWordListening(phrase)
      : c.wakeWordOff(phrase)

  const tooltip = !pausedForVoice && wake.notice ? `${label} — ${wake.notice}` : label

  return (
    <Tip label={tooltip} placement="control">
      <Button
        aria-label={label}
        aria-pressed={wake.listening && !pausedForVoice}
        className={cn(GHOST_ICON_BTN, 'p-0', wake.listening && !pausedForVoice && ACTIVE_ICON_BTN)}
        disabled={disabled || pausedForVoice || wake.pending}
        onClick={() => {
          triggerHaptic(wake.listening ? 'close' : 'open')
          void toggleWakeWord()
        }}
        size="icon"
        type="button"
        variant="ghost"
      >
        {wake.listening && !pausedForVoice ? <Ear className={iconSize.sm} /> : <EarOff className={iconSize.sm} />}
      </Button>
    </Tip>
  )
}
