import { group, mirrorTreeHorizontal, split } from '@/components/pane-shell/tree/model'
import { registerBundledPresets } from '@/components/pane-shell/tree/presets'

// ---------------------------------------------------------------------------
// Layout presets — CHAT (main) always dominates.
// ---------------------------------------------------------------------------

// The REAL default: sessions left, chat main, and a column of tool CARDS on the right. Terminal, Files, Changes and
// (docked above Files when it is opened) the Browser are separate cards, each toggled from the pane cluster in the chat
// header. A card that is closed is hidden and its siblings absorb the space; when none is open the chat has the whole
// width. Every tool is one click away and several can be open together.
//
// Preview tiles are DYNAMIC panes (like session tiles), so no preset names one: they're registered by
// watchPreviewTiles as tabs open and dock above the Files card.
export const TOOLS_COLUMN = split(
  'column',
  [group(['terminal'], { id: 'grp-terminal' }), group(['files'], { id: 'grp-files' }), group(['review'], { id: 'grp-review' })],
  [1, 1, 1],
  'spl-tools'
)

export const DEFAULT_TREE = split(
  'row',
  [group(['sessions'], { id: 'grp-sessions' }), group(['workspace'], { id: 'grp-main' }), TOOLS_COLUMN],
  [1, 3.4, 1.25],
  'spl-root'
)

// The simple tier's arrangement is the default one: sessions, chat and the tool cards (all closed until opened).
export const BASIC_TREE = DEFAULT_TREE

// Every arrangement below is built from the same panes: Sessions, the chat, and the Terminal / Files / Changes cards.
// `resting` is what a preset leaves closed; everything else it places opens when it is applied.
const BASIC_RESTING = ['terminal', 'files', 'review'] as const

// Terminal under the chat; Files and Changes stay a column on the right.
const TERMINAL_BELOW_TREE = split(
  'row',
  [
    group(['sessions'], { id: 'grp-sessions' }),
    split('column', [group(['workspace'], { id: 'grp-main' }), group(['terminal'], { id: 'grp-terminal' })], [3, 1.1]),
    split('column', [group(['files'], { id: 'grp-files' }), group(['review'], { id: 'grp-review' })], [1, 1])
  ],
  [1, 3.4, 1.25]
)

// The tool cards between Sessions and the chat.
const TOOLS_LEFT_TREE = split(
  'row',
  [group(['sessions'], { id: 'grp-sessions' }), TOOLS_COLUMN, group(['workspace'], { id: 'grp-main' })],
  [1, 1.25, 3.4]
)

// The chat on top, Terminal and Changes side by side beneath it, Files on the right.
const GRID_TREE = split(
  'row',
  [
    group(['sessions'], { id: 'grp-sessions' }),
    split(
      'column',
      [
        group(['workspace'], { id: 'grp-main' }),
        split('row', [group(['terminal'], { id: 'grp-terminal' }), group(['review'], { id: 'grp-review' })], [1.4, 1])
      ],
      [3, 1.1]
    ),
    group(['files'], { id: 'grp-files' })
  ],
  [1, 3.4, 1.1]
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
    { id: 'default', title: 'Default', order: 0, tree: DEFAULT_TREE, resting: BASIC_RESTING, tier: 'advanced' },
    { id: 'tools-open', title: 'Tools open', order: 5, tree: DEFAULT_TREE, tier: 'advanced' },
    { id: 'terminal-below', title: 'Terminal below', order: 10, tree: TERMINAL_BELOW_TREE, resting: ['files', 'review'], tier: 'advanced' },
    { id: 'tools-left', title: 'Tools left', order: 20, tree: TOOLS_LEFT_TREE, resting: BASIC_RESTING, tier: 'advanced' },
    { id: 'grid', title: 'Grid', order: 30, tree: GRID_TREE, tier: 'advanced' }
  ])
}
