import { useEffect, useState } from 'react'
import { useLocation, useNavigate, useNavigationType } from 'react-router'

/** Position of the current entry in the router's own history stack (HashRouter
 *  keeps it in `history.state.idx`). */
const currentIndex = () => (window.history.state as null | { idx?: number })?.idx ?? 0

/**
 * Back / forward over the app's route history. The browser does not say how
 * many forward entries exist, so the deepest index seen is tracked here; a
 * PUSH truncates the forward stack, exactly as the browser does.
 */
export function useNavHistory() {
  const navigate = useNavigate()
  const location = useLocation()
  const type = useNavigationType()
  const [deepest, setDeepest] = useState(currentIndex)
  const index = currentIndex()

  useEffect(
    () => setDeepest(prev => (type === 'PUSH' ? currentIndex() : Math.max(prev, currentIndex()))),
    [location.key, type]
  )

  return {
    back: () => navigate(-1),
    canGoBack: index > 0,
    canGoForward: index < deepest,
    forward: () => navigate(1)
  }
}
