import { beforeEach, describe, expect, it } from 'vitest'

import {
  $titlebarAppActionsSide,
  setTitlebarAppActionsSide,
  TITLEBAR_APP_ACTIONS_DEFAULT,
  titlebarAppActionsClusterCounts
} from './titlebar-app-actions'

describe('titlebarAppActionsClusterCounts', () => {
  it('adds extras to the cluster they belong to', () => {
    for (const side of ['left', 'right'] as const) {
      const base = titlebarAppActionsClusterCounts(side)
      expect(titlebarAppActionsClusterCounts(side, 1, 2)).toEqual({ left: base.left + 1, right: base.right + 2 })
    }
  })

  it('releases the space of every tool Simple mode hides, on both sides', () => {
    // Back, forward and the sidebar toggle (3); settings moved to the account row and the pane cluster lives in the
    // chat header, so the top bar carries no fixed app actions on either side.
    expect(titlebarAppActionsClusterCounts('right', 0, 0, 'simple')).toEqual({ left: 3, right: 0 })
    expect(titlebarAppActionsClusterCounts('left', 0, 0, 'simple')).toEqual({ left: 3, right: 0 })
  })
})

describe('$titlebarAppActionsSide', () => {
  beforeEach(() => {
    window.localStorage.clear()
    setTitlebarAppActionsSide(TITLEBAR_APP_ACTIONS_DEFAULT)
  })

  it('persists left', () => {
    setTitlebarAppActionsSide('left')
    expect($titlebarAppActionsSide.get()).toBe('left')
    expect(window.localStorage.getItem('factr.desktop.titlebarAppActions')).toBe('left')
  })
})
