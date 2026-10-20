import { describe, expect, it } from 'vitest'

import { group, isToolCardPane, type LayoutNode, migratePersistedTree, split } from './model'
import { $maximizedPane, toggleMaximizedPane } from './pane-maximize'

const shape = (node: LayoutNode): unknown =>
  node.type === 'group' ? node.panes : { [node.orientation]: node.children.map(shape) }

describe('tool cards', () => {
  it('recognises the tool panes and browser tabs, and nothing else', () => {
    for (const id of ['terminal', 'files', 'review', 'preview-tile:abc']) {
      expect(isToolCardPane(id)).toBe(true)
    }

    for (const id of ['workspace', 'sessions', 'bots', 'session-tile:1']) {
      expect(isToolCardPane(id)).toBe(false)
    }
  })

  it('migrates the old tabbed right sidebar into one card per tool, keeping the first group id', () => {
    const legacy = split(
      'row',
      [group(['sessions']), group(['workspace']), group(['terminal', 'files', 'review'], { id: 'grp-right' })],
      [1, 3.4, 1.25],
      'spl-root'
    )

    const migrated = migratePersistedTree(legacy)

    expect(shape(migrated)).toEqual({ row: [['sessions'], ['workspace'], { column: [['terminal'], ['files'], ['review']] }] })

    const column = (migrated as { children: LayoutNode[] }).children[2] as { children: { id: string }[] }

    expect(column.children[0]!.id).toBe('grp-right')
    // Running it again changes nothing: a migrated tree is already one card per zone.
    expect(shape(migratePersistedTree(migrated))).toEqual(shape(migrated))
  })

  it('leaves a zone alone when a tool pane shares it with other work', () => {
    const focus = split('row', [group(['sessions']), group(['workspace', 'files', 'review'])], [1, 4])

    expect(shape(migratePersistedTree(focus))).toEqual({ row: [['sessions'], ['workspace', 'files', 'review']] })
  })

  it('fill is a single toggle and restores on the second press', () => {
    $maximizedPane.set(null)
    toggleMaximizedPane('files')
    expect($maximizedPane.get()).toBe('files')
    toggleMaximizedPane('terminal')
    expect($maximizedPane.get()).toBe('terminal')
    toggleMaximizedPane('terminal')
    expect($maximizedPane.get()).toBeNull()
  })
})
