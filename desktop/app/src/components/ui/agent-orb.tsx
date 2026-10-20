import { ThinkingOrb } from 'thinking-orbs'

import { useMediaQuery } from '@/hooks/use-media-query'
import { useTheme } from '@/themes'

/** One orb for engine boot only. Inline work uses Loader. */
export function AgentOrb({ paused = false }: { paused?: boolean }) {
  const { renderedMode, theme } = useTheme()
  const reduce = useMediaQuery('(prefers-reduced-motion: reduce)')

  return (
    <span aria-hidden="true" className="engine-orb" data-paused={paused || reduce ? 'true' : undefined}>
      <ThinkingOrb
        aria-hidden="true"
        color={theme.colors.foreground}
        paused={paused || reduce}
        size={64}
        speed={0.35}
        state="connecting"
        theme={renderedMode}
      />
    </span>
  )
}
