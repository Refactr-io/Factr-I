import { expect, it } from 'vitest'

import { group, split } from './model'
import { reflowToolColumns } from './tool-columns'

const tree = () =>
  split('row', [
    group(['sessions'], { id: 'grp-sessions' }),
    group(['workspace'], { id: 'grp-main' }),
    split('column', [group(['terminal'], { id: 'grp-terminal' }), group(['files'], { id: 'grp-files' }), group(['review'], { id: 'grp-review' })], [1, 1, 1], 'spl-tools')
  ], [1, 3.4, 1.25])

const columns = (root: ReturnType<typeof tree>) =>
  root.children.slice(2).map(c => (c.type === 'split' ? c.children.map(g => g.id) : [c.id]))

const only = (...ids: string[]) => (pane: string) => ids.includes(pane)

it('keeps up to two open cards in one column', () => {
  const t = tree()
  expect(reflowToolColumns(t, only('terminal', 'review'))).toBe(t)
  expect(reflowToolColumns(t, only())).toBe(t)
})

it('starts a new column for the third and fourth card', () => {
  const next = reflowToolColumns(tree(), only('terminal', 'files', 'review')) as ReturnType<typeof tree>
  expect(columns(next)).toEqual([['grp-terminal', 'grp-files'], ['grp-review']])
  expect(next.weights.length).toBe(next.children.length)
})

it('puts closed cards after the visible ones and folds back when one closes', () => {
  const three = reflowToolColumns(tree(), only('terminal', 'files', 'review')) as ReturnType<typeof tree>
  expect(reflowToolColumns(three, only('terminal', 'files', 'review'))).toBe(three)
  const two = reflowToolColumns(three, only('terminal', 'review')) as ReturnType<typeof tree>
  expect(columns(two)).toHaveLength(1)
  expect(columns(two)[0]).toHaveLength(3)
})

it('mirrors the column order when the sessions column is on the right', () => {
  const t = tree()
  const mirrored = { ...t, children: [...t.children].reverse(), weights: [...t.weights].reverse() }
  const next = reflowToolColumns(mirrored, only('terminal', 'files', 'review')) as ReturnType<typeof tree>
  expect(next.children.slice(0, 2).map(c => (c.type === 'split' ? c.children.map(g => g.id) : [c.id]))).toEqual([['grp-review'], ['grp-terminal', 'grp-files']])
})

it('leaves a layout without side-by-side tool cards alone', () => {
  const t = split('row', [group(['workspace'], { id: 'grp-main' }), group(['terminal'], { id: 'a' }), group(['x'], { id: 'b' }), group(['files'], { id: 'c' })])
  expect(reflowToolColumns(t, only('terminal', 'files'))).toBe(t)
})
