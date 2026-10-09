import { group, mirrorTreeHorizontal, split } from '@/components/pane-shell/tree/model'
import { registerBundledPresets } from '@/components/pane-shell/tree/presets'

// ---------------------------------------------------------------------------
// Layout presets — CHAT (main) always dominates.
// ---------------------------------------------------------------------------

// The REAL default: sessions left, chat main, and ONE right sidebar whose tab strip carries Terminal,
// Files and Changes (the review pane), with Browser stacking in beside them when it is opened. Every tool
// is one click away; the sidebar's toggle shows or hides the whole thing.
//
// Preview tiles are DYNAMIC panes (like session tiles), so no preset names one: they're registered by
// watchPreviewTiles as tabs open. A Browser tab stacks into this sidebar; a file peek still opens its own
// pane beside it.
export const DEFAULT_TREE = split(
  'row',
  [
    group(['sessions'], { id: 'grp-sessions' }),
    group(['workspace'], { id: 'grp-main' }),
    group(['terminal', 'files', 'review'], { id: 'grp-right' })
  ],
  [1, 3.4, 1.25],
  'spl-root'
)

// Focus is one column of attention: files and review are tabs BEHIND the chat,
// the terminal a collapsed rail under it — opening the terminal must never
// cover the conversation, which a terminal tab did.
const FOCUS_TREE = split(
  'row',
  [group(['sessions']), split('column', [group(['workspace', 'files', 'review']), group(['terminal'])], [3, 1])],
  [1, 4.6]
)

// Basic is sessions and chat with the tooling RESTING in one right sidebar: a single zone whose
// tab strip carries Terminal, Files and Changes (and Browser, which stacks in when opened), so every
// tool is one click away instead of its own toggle. The toggle shows or hides the whole sidebar.
// A tree that simply omitted the tools was a lie — applying it adopts every missing pane back in as
// workspace tabs, which is Focus.
export const BASIC_TREE = split(
  'row',
  [group(['sessions']), group(['workspace']), group(['terminal', 'files', 'review'], { id: 'grp-right' })],
  [1, 3.4, 1.25]
)

const BASIC_RESTING = ['terminal', 'files', 'review'] as const

const TERMINAL_TREE = split(
  'column',
  [
    split('row', [group(['sessions']), group(['workspace']), group(['files', 'review'])], [1, 3.2, 1.2]),
    group(['terminal'])
  ],
  [3, 1]
)

const QUAD_TREE = split(
  'column',
  [
    split('row', [group(['sessions', 'files']), group(['workspace'])], [1, 3]),
    split('row', [group(['terminal']), group(['review'])], [1.4, 1])
  ],
  [3, 1]
)

export function registerLayoutPresets() {
  // Simple is always the Basic arrangement; its one choice is which side the
  // sidebar sits. The decks are Advanced — arranging tooling is the point.
  return registerBundledPresets([
    { id: 'sidebar-left', title: 'Sidebar left', order: 0, tree: BASIC_TREE, resting: BASIC_RESTING, tier: 'simple' },
    {
      id: 'sidebar-right',
      title: 'Sidebar right',
      order: 1,
      tree: mirrorTreeHorizontal(BASIC_TREE),
      resting: BASIC_RESTING,
      tier: 'simple'
    },
    { id: 'default', title: 'Default', order: 0, tree: DEFAULT_TREE, tier: 'advanced' },
    { id: 'basic', title: 'Basic', order: 5, tree: BASIC_TREE, resting: BASIC_RESTING, tier: 'advanced' },
    { id: 'focus', title: 'Focus', order: 10, tree: FOCUS_TREE, resting: ['terminal'], tier: 'advanced' },
    { id: 'terminal-deck', title: 'Terminal deck', order: 20, tree: TERMINAL_TREE, tier: 'advanced' },
    { id: 'quad', title: 'Quad', order: 30, tree: QUAD_TREE, tier: 'advanced' }
  ])
}
