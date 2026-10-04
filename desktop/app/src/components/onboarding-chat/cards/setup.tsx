/**
 * The setup cards: appearance and layout. The picks apply as soon as they are clicked. The option lists come from onboarding-chat/options.tsx, so the cards
 * and the previews stay in agreement without the model listing the options.
 */

import { useStore } from '@nanostores/react'

import { $chatLayoutPicked, assembleChatOnboarding } from '@/components/onboarding-chat/assembly'
import { CardFrame, type CardProps, useCardCommit } from '@/components/onboarding-chat/cards/frame'
import { LayoutPreviewCard, LAYOUTS } from '@/components/onboarding-chat/options'
import type { LayoutNode } from '@/components/pane-shell/tree/model'
import { registry } from '@/contrib/registry'
import { cn } from '@/lib/utils'
import { $onboardingAnswers, setOnboardingAnswers } from '@/store/onboarding-answers'
import { useTheme } from '@/themes'

export function LookCard({ attrs, locked }: CardProps) {
  const { mode, setMode } = useTheme()
  const { commit, done } = useCardCommit('look')

  if (attrs.value !== undefined) {
    return null
  }

  return (
    <CardFrame done={done} locked={locked} onContinue={() => commit(`appearance: ${mode}`)}>
      <div className="flex gap-2.5">
        {(['light', 'dark'] as const).map(choice => (
          <button
            aria-pressed={mode === choice}
            className={cn(
              'min-w-24 border px-4 py-3 text-sm font-medium',
              mode === choice ? 'border-(--ui-text-primary)' : 'border-(--ui-stroke-secondary)'
            )}
            key={choice}
            onClick={() => setMode(choice)}
            type="button"
          >
            {choice === 'light' ? 'White' : 'Black'}
          </button>
        ))}
      </div>
    </CardFrame>
  )
}

export function LayoutCard({ locked }: CardProps) {
  const answers = useStore($onboardingAnswers)
  const { commit, done } = useCardCommit('layout')
  // The stored answer defaults to 'basic', so nothing renders selected and Continue stays disabled until the user
  // clicks. The flag lives in a store because applying the picked layout replaces the pane tree and remounts this
  // card, which would clear local state.
  const picked = useStore($chatLayoutPicked)

  const pickLayout = (id: string) => {
    $chatLayoutPicked.set(true)
    setOnboardingAnswers({ layout: id })

    // The pick answers "how much of the machinery do you want to see" too;
    // Skip leaves the mode alone, so only an actual choice sets it.
    const layout = LAYOUTS.find(candidate => candidate.id === id)

    const preset = registry.getArea('layouts').find(contribution => contribution.id === id)

    if (!preset?.data) {
      return
    }

    // Every pick goes through assembly, including re-picks. The first pick grows the window and places the panes,
    // holding the chat and the cursor over this card at the same screen position; later picks rearrange in place.
    // Swapping only the preset tree on a re-pick kept the previous layout's dismissals and dock records, and the two
    // layouts came up mixed together.
    // SAFETY: Layout presets declare data: LayoutNode (pane-shell/tree/presets.ts).
    assembleChatOnboarding(preset.id, preset.data as LayoutNode, layout?.mode)
  }

  return (
    <CardFrame
      disabled={!picked}
      done={done}
      locked={locked}
      onContinue={() => {
        const choice = LAYOUTS.find(layout => layout.id === answers.layout)

        commit(`layout: ${choice?.name ?? answers.layout}`)
      }}
    >
      <div className="grid grid-cols-2 gap-3">
        {LAYOUTS.map(layout => (
          <LayoutPreviewCard
            active={picked && answers.layout === layout.id}
            description={layout.description}
            key={layout.id}
            name={layout.name}
            onSelect={() => pickLayout(layout.id)}
            tree={layout.tree}
          />
        ))}
      </div>
    </CardFrame>
  )
}
