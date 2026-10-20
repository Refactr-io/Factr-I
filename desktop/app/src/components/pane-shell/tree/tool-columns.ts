import { allPaneIds, type GroupNode, isToolCardPane, type LayoutNode, nodeId, split } from './model'

/**
 * Tool cards (Terminal, Changes, Browser tabs, Files) open at most TWO to a column, like Claude's panels: the third
 * card starts a new column beside the first instead of squeezing a fourth sliver into the same one. Pure: takes the
 * root row, answers with a tree whose tool cards sit in that arrangement (or the same object when it already does).
 * Cards that are closed stay in the tree (they cost no space), after the visible ones of the last column.
 */
const CARDS_PER_COLUMN = 2

const isToolOnly = (node: LayoutNode): node is GroupNode =>
  node.type === 'group' && node.panes.length > 0 && node.panes.every(isToolCardPane)

export function reflowToolColumns(root: LayoutNode, isVisible: (paneId: string) => boolean): LayoutNode {
  if (root.type !== 'split' || root.orientation !== 'row') {
    return root
  }

  // Swapped layouts keep the sessions column after the chat; the tool columns mirror with it.
  const at = (paneId: string) => root.children.findIndex(child => allPaneIds(child).includes(paneId))
  const flipped = at('sessions') > at('workspace') && at('workspace') >= 0

  const slots: { groups: GroupNode[]; id: string; index: number; weight: number }[] = []

  root.children.forEach((child, index) => {
    const weight = root.weights[index] ?? 1

    if (isToolOnly(child)) {
      slots.push({ groups: [child], id: child.id, index, weight })
    } else if (child.type === 'split' && child.orientation === 'column' && child.children.every(isToolOnly)) {
      slots.push({ groups: child.children as GroupNode[], id: child.id, index, weight })
    }
  })

  // Only the standard shape: tool cards as (columns of) direct children of the root row, side by side.
  if (slots.length === 0 || slots.some((s, i) => s.index !== slots[0]!.index + i)) {
    return root
  }

  // Act only when a column holds more than two open cards, or to fold back columns this module made (their column
  // splits carry the 'spl-tools-' id) once few enough are open. Any other arrangement is the user's own.
  const over = slots.some(s => s.groups.filter(g => g.panes.some(isVisible)).length > CARDS_PER_COLUMN)
  const ours = slots.length > 1 && slots.some(s => s.groups.length > 1 && s.id.startsWith('spl-tools-'))

  if (!over && !ours) {
    return root
  }

  // Reading order: left to right in the unswapped layout (the mirrored one reads right to left), top to bottom.
  const ordered = (flipped ? [...slots].reverse() : slots).flatMap(s => s.groups)
  const visible = ordered.filter(g => g.panes.some(isVisible))
  const hidden = ordered.filter(g => !g.panes.some(isVisible))
  // Already one column and two or fewer open: leave the user's order alone.
  if (visible.length <= CARDS_PER_COLUMN && slots.length === 1) {
    return root
  }

  const chunks: GroupNode[][] = []

  for (let i = 0; i < visible.length; i += CARDS_PER_COLUMN) {
    chunks.push(visible.slice(i, i + CARDS_PER_COLUMN))
  }

  if (chunks.length === 0) {
    chunks.push([])
  }

  chunks[chunks.length - 1] = [...chunks[chunks.length - 1]!, ...hidden]

  if (flipped) {
    chunks.reverse()
  }

  const key = (columns: GroupNode[][]) => columns.map(c => c.map(g => g.id).join('+')).join('|')
  const current = slots.map(s => s.groups)

  if (key(current) === key(chunks)) {
    return root
  }

  // A second column should not take a second full share from the chat: each of several columns is a little slimmer.
  const average = (slots.reduce((sum, s) => sum + s.weight, 0) / slots.length) * (chunks.length > 1 ? 0.8 : 1)

  const columns: LayoutNode[] = chunks.map(chunk =>
    chunk.length === 1 ? chunk[0]! : split('column', chunk, undefined, nodeId('spl-tools'))
  )

  const first = slots[0]!.index
  const children = [...root.children.slice(0, first), ...columns, ...root.children.slice(first + slots.length)]

  const weights = [
    ...root.weights.slice(0, first),
    ...columns.map(() => average),
    ...root.weights.slice(first + slots.length)
  ]

  return { ...root, children, weights }
}
