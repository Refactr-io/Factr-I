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

// Focus is one column of attention: files and review are tabs BEHIND the chat,
// the terminal a collapsed rail under it — opening the terminal must never
// cover the conversation, which a terminal tab did.
const FOCUS_TREE = split(
  'row',
  [group(['sessions']), split('column', [group(['workspace', 'files', 'review']), group(['terminal'])], [3, 1])],
  [1, 4.6]
)

// Basic is the same arrangement as Default: sessions, chat and the tool cards (all closed until opened).
export const BASIC_TREE = DEFAULT_TREE

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
