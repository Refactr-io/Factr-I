import { cleanup, fireEvent, render } from '@testing-library/react'
import { atom } from 'nanostores'
import { useEffect } from 'react'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { ContribRender } from '@/contrib/react/boundary'
import { registry } from '@/contrib/registry'

import { paneMirror } from './pane-mirror'

afterEach(cleanup)

describe('pane title refresh lifecycle', () => {
  it('keeps mounted content and local state when only the title changes', () => {
    const tiles = atom([{ id: 'one', title: 'Browser' }])
    const mounted = vi.fn()
    const unmounted = vi.fn()

    function Body() {
      useEffect(() => {
        mounted()

        return unmounted
      }, [])

      return <input aria-label="Local draft" defaultValue="" />
    }

    const watch = paneMirror({
      source: tiles,
      prefix: 'test-pane-title',
      minWidth: '240px',
      key: tile => tile.id,
      title: id => tiles.get().find(tile => tile.id === id)!.title,
      render: () => <Body />,
      close: () => undefined
    })

    watch()

    const contribution = () => registry.getArea('panes').find(pane => pane.id === 'test-pane-title:one')!
    const view = render(<ContribRender render={contribution().render!} />)
    fireEvent.change(view.getByRole('textbox'), { target: { value: 'Keep this draft' } })
    tiles.set([{ id: 'one', title: 'Loaded page title' }])
    expect(contribution().title).toBe('Loaded page title')
    view.rerender(<ContribRender render={contribution().render!} />)

    expect(mounted).toHaveBeenCalledOnce()
    expect(unmounted).not.toHaveBeenCalled()
    expect((view.getByRole('textbox') as HTMLInputElement).value).toBe('Keep this draft')

    view.unmount()
    tiles.set([])
    expect(contribution()).toBeUndefined()
    expect(unmounted).toHaveBeenCalledOnce()
  })
})
