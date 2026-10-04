import { selectableClass } from '@/components/onboarding-chat/chip'
import { IS_MAC } from '@/lib/keybinds/combo'
import { cn } from '@/lib/utils'
import type { InterfaceMode } from '@/store/interface-mode'

// These mini trees copy the basic (BASIC_TREE) and terminal-deck
// (TERMINAL_TREE) presets in app/contrib/layout-presets.ts, drawn like the
// layout editor's thumbnails at a larger size.
export type MiniNode = 1 | { dir: 'column' | 'row'; children: MiniNode[]; weights: number[] }

export const ELITE_LAYOUT_ID = 'terminal-deck'

// Each pick is an arrangement AND an interface mode. First launch is the one
// place a single question can answer both: someone here to talk to Factr
// should not have to find Simple mode afterwards, and a developer who asked
// for the terminal deck wants the tooling on. Basic applies Simple's own
// preset so its shelf shows the pick as active.
export const LAYOUTS: Array<{ description: string; id: string; mode: InterfaceMode; name: string; tree: MiniNode }> = [
  {
    description: 'For talking to Factr-I.',
    id: 'sidebar-left',
    mode: 'simple',
    name: 'Basic',
    tree: { children: [1, 1], dir: 'row', weights: [1, 4.6] }
  },
  {
    description: 'For developers: terminal, files, diffs.',
    id: ELITE_LAYOUT_ID,
    mode: 'advanced',
    name: 'Elite',
    tree: {
      children: [{ children: [1, 1, 1], dir: 'row', weights: [1, 3.2, 1.2] }, 1],
      dir: 'column',
      weights: [3, 1]
    }
  }
]

export function MiniTree({ node }: { node: MiniNode }) {
  if (node === 1) {
    return <div className="min-h-0 min-w-0 flex-1 rounded-sm bg-foreground/15" />
  }

  return (
    <div className={cn('flex min-h-0 min-w-0 flex-1 gap-1', node.dir === 'row' ? 'flex-row' : 'flex-col')}>
      {node.children.map((child, i) => (
        <div className="flex min-h-0 min-w-0" key={i} style={{ flex: `${node.weights[i]} ${node.weights[i]} 0px` }}>
          <MiniTree node={child} />
        </div>
      ))}
    </div>
  )
}

/**
 * The window buttons on the preview, drawn the way this machine draws them, so the card matches the user's own window.
 * `main.ts` makes the same split: macOS puts the traffic lights on the left (`trafficLightPosition`), every other
 * platform puts monochrome native controls on the right (`titleBarOverlay`).
 */
function MiniWindowButtons() {
  if (IS_MAC) {
    return (
      <span aria-hidden className="flex gap-1">
        <span className="size-1.5 rounded-full bg-[#ff5f57]" />
        <span className="size-1.5 rounded-full bg-[#febc2e]" />
        <span className="size-1.5 rounded-full bg-[#28c840]" />
      </span>
    )
  }

  // Minimize, maximize, close. At 6 px the real glyphs are illegible, so each
  // one is a plain shape: a bar, a box, and a cross.
  return (
    <span aria-hidden className="flex items-center justify-end gap-1.5 text-foreground/40">
      <span className="h-px w-1.5 bg-current" />
      <span className="size-1.5 border border-current" />
      <span className="relative size-1.5">
        <span className="absolute top-1/2 left-0 h-px w-full rotate-45 bg-current" />
        <span className="absolute top-1/2 left-0 h-px w-full -rotate-45 bg-current" />
      </span>
    </span>
  )
}

export function LayoutPreviewCard({
  active,
  description,
  name,
  onSelect,
  tree
}: {
  active: boolean
  description?: string
  name: string
  onSelect: () => void
  tree: MiniNode
}) {
  return (
    <button aria-pressed={active} className="group flex flex-col items-center gap-2" onClick={onSelect} type="button">
      <span className={cn('flex aspect-[10/7] w-full flex-col gap-1.5 rounded-lg p-2', selectableClass(active))}>
        <MiniWindowButtons />
        <span className="flex min-h-0 flex-1">
          <MiniTree node={tree} />
        </span>
      </span>
      <span className="flex flex-col items-center gap-0.5">
        <span className={cn('text-xs', active ? 'text-foreground' : 'text-muted-foreground')}>{name}</span>
        {description && <span className="text-[0.6875rem] text-muted-foreground/90">{description}</span>}
      </span>
    </button>
  )
}
