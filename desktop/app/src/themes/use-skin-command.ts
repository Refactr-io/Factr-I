import { useCallback } from 'react'

import { useTheme } from './context'

// Keep the historical slash name for muscle memory, but it only changes the
// two fixed appearances. Arbitrary skin names are never passed to the renderer.
export function useSkinCommand() {
  const { mode, setMode } = useTheme()

  return useCallback(
    (rawArg: string) => {
      const arg = rawArg.trim().toLowerCase()

      if (arg === 'list' || arg === 'ls' || arg === 'status') {
        return `Appearance: ${mode}. Available: light, dark.`
      }

      const next = !arg || arg === 'next' ? (mode === 'light' ? 'dark' : 'light') : arg

      if (next !== 'light' && next !== 'dark') {
        return `Unknown appearance: ${rawArg.trim()}. Available: light, dark.`
      }

      setMode(next)

      return `Appearance switched to ${next}.`
    },
    [mode, setMode]
  )
}
