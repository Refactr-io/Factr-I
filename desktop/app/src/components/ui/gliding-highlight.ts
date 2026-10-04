import * as React from 'react'

/**
 * GlideSelect's shared hover pill (reactbits.dev): one pill per open list that
 * slides to the highlighted row, instead of each row painting its own fill.
 *
 * Attach the returned callback as the ref of the list's positioned container.
 * `row` matches the highlighted row (Radix: `[data-highlighted]`, cmdk:
 * `[data-selected=true]`); `attr` is the attribute that moves between rows.
 * Menus mount only while open, so a ref callback (not an effect on a
 * component that stays mounted) is what runs once per open list.
 */
export function useGlidingHighlight(row: string, attr: string): React.RefCallback<HTMLElement> {
  return React.useCallback(
    (host: HTMLElement | null) => {
      if (!host) {
        return
      }

      // The pill and every row offset are measured from the host.
      if (getComputedStyle(host).position === 'static') {
        host.style.position = 'relative'
      }

      host.setAttribute('data-glide-host', '')
      const pill = document.createElement('span')
      pill.className = 'glide-pill'
      pill.setAttribute('aria-hidden', 'true')
      host.prepend(pill)

      let frame = 0

      const sync = () => {
        const target = host.querySelector<HTMLElement>(row)

        if (!target) {
          pill.removeAttribute('data-visible')

          return
        }

        // Layout offsets, not client rects: menus open with a scale-in, and a
        // rect taken mid-animation would park the pill off its row.
        let x = 0
        let y = 0

        for (let el: HTMLElement | null = target; el && el !== host; el = el.offsetParent as HTMLElement | null) {
          x += el.offsetLeft
          y += el.offsetTop
        }

        pill.style.width = `${target.offsetWidth}px`
        pill.style.height = `${target.offsetHeight}px`
        pill.style.borderRadius = getComputedStyle(target).borderTopLeftRadius
        pill.style.transform = `translate(${x}px, ${y}px)`
        pill.setAttribute('data-visible', '')

        // Glide only after the first placement, so it opens on the current row.
        if (!frame && !pill.hasAttribute('data-glide')) {
          frame = requestAnimationFrame(() => pill.setAttribute('data-glide', ''))
        }
      }

      sync()
      const observer = new MutationObserver(sync)
      observer.observe(host, { attributes: true, attributeFilter: [attr], subtree: true })

      // React 19: a ref callback may return its cleanup.
      return () => {
        observer.disconnect()
        cancelAnimationFrame(frame)
        pill.remove()
      }
    },
    [row, attr]
  )
}

/** The highlighted row of a Radix menu (items, checkbox/radio items, sub-triggers). */
export const MENU_ROW = "[role^='menuitem'][data-highlighted]"

/** The highlighted row of a cmdk list. */
export const COMMAND_ROW = "[cmdk-item][data-selected='true']"

/** One ref callback for the glide plus a caller's own ref (object or callback). */
export function useMergedRef<T>(own: React.RefCallback<T>, other: React.Ref<T> | undefined): React.RefCallback<T> {
  return React.useCallback(
    (node: T | null) => {
      const cleanup = own(node)

      if (typeof other === 'function') {
        other(node)
      } else if (other) {
        other.current = node
      }

      return () => {
        if (typeof cleanup === 'function') {
          cleanup()
        }

        if (typeof other === 'function') {
          other(null)
        } else if (other) {
          other.current = null
        }
      }
    },
    [own, other]
  )
}
