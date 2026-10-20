import { computed } from 'nanostores'

import { allPaneIds } from '@/components/pane-shell/tree/model'
import { $layoutTree, bindTreeSideVisibility, mirrorLayoutTree } from '@/components/pane-shell/tree/store'
import { modeLayout } from '@/store/interface-mode'
import { $panesFlipped, $sidebarOpen, setSidebarOpen } from '@/store/layout'

/** Side toggles belong to panes, not physical edges. Derive the flip from the
 * tree so dragging sessions or choosing a mirrored preset remaps the buttons. */
export function bindLayoutSides() {
  const sessionsOnRight = () => {
    const tree = $layoutTree.get()

    if (!tree) {
      return null
    }

    const order = allPaneIds(tree)
    const sessions = order.indexOf('sessions')
    const main = order.indexOf('workspace')

    return sessions >= 0 && main >= 0 ? sessions > main : null
  }

  $layoutTree.subscribe(() => {
    const flipped = sessionsOnRight()

    if (flipped !== null && flipped !== $panesFlipped.get()) {
      $panesFlipped.set(flipped)
    }
  })

  $panesFlipped.listen(flipped => {
    const current = sessionsOnRight()

    // Restoration replaces the tree; a transient mismatch is not a flip gesture.
    if (!modeLayout.restoring && current !== null && current !== flipped) {
      mirrorLayoutTree()
    }
  })

  // The sessions column is the only side with a collapse toggle now. Tool cards (terminal, files, changes, browser)
  // open and close one by one, so the tool side is never collapsed as a whole.
  const $leftOpen = computed([$panesFlipped, $sidebarOpen], (flipped, sidebar) => (flipped ? true : sidebar))
  const $rightOpen = computed([$panesFlipped, $sidebarOpen], (flipped, sidebar) => (flipped ? sidebar : true))

  bindTreeSideVisibility('left', $leftOpen, open => {
    if (!$panesFlipped.get()) {
      setSidebarOpen(open)
    }
  })
  bindTreeSideVisibility('right', $rightOpen, open => {
    if ($panesFlipped.get()) {
      setSidebarOpen(open)
    }
  })
}
