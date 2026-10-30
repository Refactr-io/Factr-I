/**
 * How far above the window's bottom edge the bottom-right toast stack must sit so it never covers the composer.
 * The stack is portaled to <body>, so it cannot read the composer's own measured-height variable; it measures the
 * docked composers instead. Pure on purpose: rects in, pixels out.
 */

export interface ClearanceRect {
  bottom: number
  height: number
  left: number
  right: number
  top: number
  width: number
}

export const TOAST_BASE_OFFSET_PX = 16
const TOAST_GAP_ABOVE_COMPOSER_PX = 12

/** `toastLeft`/`toastRight` are the stack's horizontal extent. A composer that does not share any of it is no obstacle. */
export function toastBottomOffset(
  composers: readonly ClearanceRect[],
  viewportHeight: number,
  toastLeft: number,
  toastRight: number
): number {
  let offset = TOAST_BASE_OFFSET_PX

  for (const rect of composers) {
    const visible = rect.width > 0 && rect.height > 0 && rect.bottom > 0 && rect.top < viewportHeight
    const overlapsAcross = rect.left < toastRight && rect.right > toastLeft

    if (visible && overlapsAcross) {
      offset = Math.max(offset, Math.ceil(viewportHeight - rect.top + TOAST_GAP_ABOVE_COMPOSER_PX))
    }
  }

  return offset
}
