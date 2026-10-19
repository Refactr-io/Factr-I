import { useStore } from '@nanostores/react'
import { useState } from 'react'

import type { LayoutNode } from '@/components/pane-shell/tree/model'
import {
  applyLayoutPreset,
  deleteUserPreset,
  isUserPreset,
  layoutPresetResting,
  layoutPresetTier,
  LAYOUTS_AREA,
  saveCurrentLayoutAs
} from '@/components/pane-shell/tree/presets'
import { $activePresetId, resetLayoutTree } from '@/components/pane-shell/tree/store'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { useContributions } from '@/contrib/react/use-contributions'
import { useI18n } from '@/i18n'
import { triggerHaptic } from '@/lib/haptics'
import { cn } from '@/lib/utils'
import { $interfaceMode, shownInMode } from '@/store/interface-mode'
import { $panesFlipped, togglePanesFlipped } from '@/store/layout'

import { ListRow, ToggleRow } from './primitives'

/** A miniature of the arrangement: the chat is the solid block, Sessions the light one, tool cards outlined. */
function Thumb({ node, resting }: { node: LayoutNode; resting: ReadonlySet<string> }) {
  if (node.type === 'split') {
    return (
      <div className={cn('flex size-full gap-0.5', node.orientation === 'row' ? 'flex-row' : 'flex-col')}>
        {node.children.map((child, i) => (
          <div className="flex min-h-0 min-w-0" key={child.id} style={{ flex: `${node.weights[i] ?? 1} 1 0px` }}>
            <Thumb node={child} resting={resting} />
          </div>
        ))}
      </div>
    )
  }

  const shut = node.panes.every(id => resting.has(id))

  const tone = node.panes.includes('workspace')
    ? 'bg-(--ui-text-quaternary)'
    : node.panes.includes('sessions')
      ? 'bg-(--ui-control-active-background)'
      : shut
        ? 'border border-dashed border-(--ui-stroke-secondary)'
        : 'border border-(--ui-text-quaternary)'

  return <div className={cn('size-full rounded-[2px]', tone)} />
}

/** Layout lives here, in Settings: pick an arrangement, swap the sidebar sides, reset, or save the current one. */
export function LayoutPicker() {
  const { t } = useI18n()
  const a = t.settings.appearance
  const mode = useStore($interfaceMode)
  const active = useStore($activePresetId)
  const flipped = useStore($panesFlipped)
  const presets = useContributions(LAYOUTS_AREA)
  const [name, setName] = useState('')

  const shown = presets
    .filter(preset => shownInMode(mode)({ tier: layoutPresetTier(preset.id) }))
    .sort((x, y) => (x.order ?? 0) - (y.order ?? 0))

  const save = () => {
    if (name.trim()) {
      saveCurrentLayoutAs(name)
      setName('')
    }
  }

  return (
    <>
      <ListRow
        below={
          <div className="mt-3 grid grid-cols-[repeat(auto-fill,minmax(9rem,1fr))] gap-3" data-layout-picker="">
            {shown.map(preset => (
              <div className="group/preset relative" key={preset.id}>
                <button
                  aria-pressed={active === preset.id}
                  className={cn(
                    'flex w-full flex-col gap-2 rounded-lg border p-2 text-left transition-colors duration-150 ease-out hover:bg-(--ui-control-hover-background) focus-visible:outline-2 focus-visible:outline-ring',
                    active === preset.id
                      ? 'border-(--ui-text-tertiary) bg-(--ui-control-active-background)'
                      : 'border-(--ui-stroke-secondary)'
                  )}
                  onClick={() => {
                    triggerHaptic('selection')
                    applyLayoutPreset(preset.id, preset.data as LayoutNode)
                  }}
                  type="button"
                >
                  <span className="block h-14 w-full">
                    <Thumb node={preset.data as LayoutNode} resting={layoutPresetResting(preset.id)} />
                  </span>
                  <span className="truncate text-xs font-medium text-foreground">{preset.title}</span>
                </button>
                {isUserPreset(preset.id) && (
                  <button
                    aria-label={a.layoutDelete(preset.title ?? preset.id)}
                    className="absolute top-1.5 right-1.5 hidden size-5 place-items-center rounded-sm text-xs text-(--ui-text-tertiary) hover:bg-(--ui-control-hover-background) hover:text-foreground group-hover/preset:grid focus-visible:grid"
                    onClick={() => deleteUserPreset(preset.id)}
                    type="button"
                  >
                    ×
                  </button>
                )}
              </div>
            ))}
          </div>
        }
        description={a.layoutDesc}
        title={a.layoutTitle}
        wide
      />
      <ToggleRow
        checked={flipped}
        description={a.layoutSwapDesc}
        label={a.layoutSwapTitle}
        onChange={() => togglePanesFlipped()}
      />
      <ListRow
        action={
          <Button onClick={() => resetLayoutTree()} size="sm" type="button" variant="outline">
            {a.layoutResetAction}
          </Button>
        }
        description={a.layoutResetDesc}
        title={a.layoutResetTitle}
      />
      <ListRow
        action={
          <form
            className="flex items-center gap-2"
            onSubmit={event => {
              event.preventDefault()
              save()
            }}
          >
            <Input
              aria-label={a.layoutSaveTitle}
              onChange={event => setName(event.target.value)}
              placeholder={a.layoutSavePlaceholder}
              size="xs"
              value={name}
            />
            <Button disabled={!name.trim()} size="sm" type="submit" variant="outline">
              {a.layoutSaveAction}
            </Button>
          </form>
        }
        description={a.layoutSaveDesc}
        title={a.layoutSaveTitle}
      />
    </>
  )
}
