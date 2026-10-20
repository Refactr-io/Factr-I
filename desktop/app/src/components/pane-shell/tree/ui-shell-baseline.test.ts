import { beforeEach, expect, it, vi } from 'vitest'

import type { LayoutNode } from './model'

beforeEach(() => {
  window.localStorage.clear()
  vi.resetModules()
})

async function boot() {
  const { atom } = await import('nanostores')
  const mode = await import('@/store/interface-mode')
  const layout = await import('@/store/layout')
  const tree = await import('./store')
  const model = await import('./model')
  const { registry } = await import('@/contrib/registry')
  const presets = await import('./presets')
  const { registerLayoutPresets, DEFAULT_TREE, BASIC_TREE } = await import('@/app/contrib/layout-presets')
  const terminal = await import('@/app/right-sidebar/store')
  const { bindLayoutSides } = await import('@/app/contrib/layout-sides')

  for (const [id, placement] of [
    ['sessions', 'left'],
    ['workspace', 'main'],
    ['files', 'right'],
    ['review', 'right'],
    ['terminal', 'bottom']
  ] as const) {
    registry.register({ id, area: 'panes', data: { placement, uncloseable: id === 'workspace' }, render: () => null })
  }

  registerLayoutPresets()
  tree.declareDefaultTree(DEFAULT_TREE, BASIC_TREE)
  tree.watchContributedPanes()
  bindLayoutSides()
  tree.bindPaneVisibility(
    'files',
    layout.$fileBrowserOpen,
    () => layout.setFileBrowserOpen(false),
    () => layout.setFileBrowserOpen(true)
  )
  const $review = atom(false)
  tree.bindPaneVisibility(
    'review',
    $review,
    () => $review.set(false),
    () => $review.set(true)
  )
  tree.bindToolPaneCollapse(
    'terminal',
    terminal.$terminalTakeover,
    () => terminal.setTerminalTakeover(false),
    () => terminal.setTerminalTakeover(true),
    mode.$showsAdvancedChrome
  )

  const apply = (id: string) => {
    const preset = registry.getArea('layouts').find(p => p.id === id)!
    presets.applyLayoutPreset(id, preset.data as LayoutNode)
  }

  return { tree, model, registry, apply }
}

// Node IDs are generated; the ordered pane stacks and split axes are the
// arrangement contract that migrations and pane commands must preserve.
function arrangement(node: LayoutNode): unknown {
  return node.type === 'group' ? node.panes : { [node.orientation]: node.children.map(arrangement) }
}

const TOOLS = { column: [['terminal'], ['files'], ['review']] }

it.each([
  ['default', { row: [['sessions'], ['workspace'], TOOLS] }, ['terminal', 'files', 'review']],
  ['tools-open', { row: [['sessions'], ['workspace'], TOOLS] }, []],
  [
    'terminal-below',
    { row: [['sessions'], { column: [['workspace'], ['terminal']] }, { column: [['files'], ['review']] }] },
    ['files', 'review']
  ],
  ['tools-left', { row: [['sessions'], TOOLS, ['workspace']] }, ['terminal', 'files', 'review']],
  [
    'grid',
    { row: [['sessions'], { column: [['workspace'], { row: [['terminal'], ['review']] }] }, ['files']] },
    []
  ]
])('applies the %s layout: arrangement, panes, and what rests', async (id, expected, resting) => {
  const { tree, model, apply } = await boot()
  apply(id as string)

  const current = tree.$layoutTree.get()!
  expect(arrangement(current)).toEqual(expected)
  expect(model.allPaneIds(current).sort()).toEqual(['files', 'review', 'sessions', 'terminal', 'workspace'])
  expect(model.findGroupOfPane(current, 'workspace')?.active).toBe('workspace')
  expect(tree.$activePresetId.get()).toBe(id)

  for (const pane of ['terminal', 'files', 'review']) {
    expect(tree.isPaneVisible(pane), pane).toBe(!(resting as string[]).includes(pane))
  }

  // The header's pill reads these stores: they must agree with what the layout put on screen.
  const { $fileBrowserOpen } = await import('@/store/layout')
  expect($fileBrowserOpen.get(), 'files store').toBe(tree.isPaneVisible('files'))

  expect(tree.isPaneVisible('workspace')).toBe(true)
  expect(tree.isPaneVisible('sessions')).toBe(true)
})

it('persists a dragged tree and does not rearrange it during late adoption', async () => {
  const { tree, model, registry, apply } = await boot()
  apply('default')
  tree.moveTreePane('files', {
    groupId: model.findGroupOfPane(tree.$layoutTree.get()!, 'workspace')!.id,
    pos: 'center'
  })
  tree.moveTreePane('terminal', {
    groupId: model.findGroupOfPane(tree.$layoutTree.get()!, 'workspace')!.id,
    pos: 'bottom'
  })
  const dragged = structuredClone(tree.$layoutTree.get()!)
  expect(tree.$activePresetId.get()).toBe('custom')
  expect([...tree.$userPlacedPanes.get()].sort()).toEqual(['files', 'terminal'])
  expect(JSON.parse(window.localStorage.getItem('factr.desktop.layoutTree.v2')!)).toEqual(dragged)

  registry.register({ id: 'late-tool', area: 'panes', data: { placement: 'right' }, render: () => null })
  const withLate = tree.$layoutTree.get()!
  const workspace = model.findGroupOfPane(dragged, 'workspace')!
  expect(model.findGroupOfPane(withLate, 'workspace')).toEqual({
    ...workspace,
    panes: [...workspace.panes, 'late-tool']
  })
  expect(model.findGroupOfPane(withLate, 'terminal')).toEqual(model.findGroupOfPane(dragged, 'terminal'))
  expect(model.findGroupOfPane(withLate, 'review')).toEqual(model.findGroupOfPane(dragged, 'review'))
  expect(model.findGroupOfPane(withLate, 'sessions')).toEqual(model.findGroupOfPane(dragged, 'sessions'))
})

it('adopts Bots as a Sessions tab and conditionally registered Routines beside the workspace', async () => {
  const { tree, model, registry, apply } = await boot()
  apply('default')
  registry.register({
    id: 'factr-bots:pane',
    area: 'panes',
    data: { placement: 'left', hideOnly: true, dock: { pane: 'sessions', pos: 'center', enforce: true } },
    render: () => null
  })
  const sessions = model.findGroupOfPane(tree.$layoutTree.get()!, 'sessions')!
  expect(sessions.panes).toEqual(['sessions', 'factr-bots:pane'])
  expect(sessions.active).toBe('sessions')
  expect(tree.tabStripVisibleForGroup(sessions)).toBe(true)
  expect(model.allPaneIds(tree.$layoutTree.get()!)).not.toContain('factr-bots:routines')

  const registerRoutines = () =>
    registry.register({
      id: 'factr-bots:routines',
      area: 'panes',
      data: {
        placement: 'main',
        dock: { pane: 'workspace', pos: 'right', enforce: true },
        defaultCollapsed: true,
        width: '250px'
      },
      render: () => null
    })

  const unregister = registerRoutines()
  const routines = model.findGroupOfPane(tree.$layoutTree.get()!, 'factr-bots:routines')!
  expect(routines.panes).toEqual(['factr-bots:routines'])
  expect(routines.minimized).toBe(true)
  expect(routines.id).not.toBe(sessions.id)

  unregister()
  expect(registry.getArea('panes').some(p => p.id === 'factr-bots:routines')).toBe(false)
  registerRoutines()
  expect(model.findGroupOfPane(tree.$layoutTree.get()!, 'factr-bots:routines')).toEqual(routines)
  expect(model.findGroupOfPane(tree.$layoutTree.get()!, 'sessions')?.active).toBe('sessions')
})

it('keeps a late Bots Screen workspace tab distinct from right tools across presets and close', async () => {
  const { tree, model, registry, apply } = await boot()
  apply('default')
  const id = 'plugin-workspace:factr-bots:screen:fixture-bot'
  registry.register({ id, area: 'panes', data: { placement: 'main', minWidth: '28rem' }, render: () => null })
  expect(model.findGroupOfPane(tree.$layoutTree.get()!, id)?.id).toBe(
    model.findGroupOfPane(tree.$layoutTree.get()!, 'workspace')?.id
  )
  expect(model.findGroupOfPane(tree.$layoutTree.get()!, 'workspace')?.active).toBe('workspace')

  for (const preset of ['tools-open', 'terminal-below', 'tools-left', 'grid', 'default']) {
    apply(preset)
    const current = tree.$layoutTree.get()!
    expect(model.allPaneIds(current).filter(p => p === id)).toHaveLength(1)
    expect(model.findGroupOfPane(current, id)?.id).toBe(model.findGroupOfPane(current, 'workspace')?.id)
  }

  tree.revealTreePane(id)
  expect(tree.isPaneVisible(id)).toBe(true)
  tree.closeTreePane(id)
  expect(model.allPaneIds(tree.$layoutTree.get()!)).not.toContain(id)
  registry.register({ id: 'late-unrelated', area: 'panes', data: { placement: 'right' }, render: () => null })
  expect(model.allPaneIds(tree.$layoutTree.get()!)).not.toContain(id)
  expect(tree.isPaneVisible('workspace')).toBe(true)
})

it('Tools open leaves visible Terminal content in the tools column', async () => {
  const { tree, model, apply } = await boot()
  apply('default')
  apply('tools-open')

  const right = model.findGroupOfPane(tree.$layoutTree.get()!, 'terminal')!
  expect(right.minimized).toBeFalsy()
  expect(tree.$hiddenTreePanes.get().has('terminal')).toBe(false)
  expect(tree.isPaneVisible('terminal')).toBe(true)
})

it('Tools left after a swap and a closed Files card: the Files store still agrees with the layout', async () => {
  const { tree, apply } = await boot()
  const layout = await import('@/store/layout')

  apply('terminal-below')
  layout.setFileBrowserOpen(true)
  tree.revealTreePane('files')
  layout.setFileBrowserOpen(false)
  layout.togglePanesFlipped()
  apply('default')
  layout.togglePanesFlipped()
  apply('tools-left')

  expect(layout.$fileBrowserOpen.get(), 'files store').toBe(tree.isPaneVisible('files'))
  expect(tree.isPaneVisible('files')).toBe(false)
})

it('a layout brings back a card that was closed (dismissed) earlier, so the tools column keeps its anchors', async () => {
  const { tree, apply } = await boot()

  apply('tools-open')
  tree.$dismissedPanes.set(new Set(['files'])) // what closing the card from the pane icons leaves behind

  apply('tools-left')

  expect(tree.$dismissedPanes.get().has('files')).toBe(false)
  expect(tree.isPaneVisible('files')).toBe(false) // the layout rests it: placed, but closed
  expect(JSON.stringify(tree.$layoutTree.get())).toContain('"files"')
})
