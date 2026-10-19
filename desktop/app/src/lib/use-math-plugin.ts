import { useEffect, useState } from 'react'

import type { createMemoizedMathPlugin } from '@/lib/katex-memo'

type MathPlugin = ReturnType<typeof createMemoizedMathPlugin>
let mathPluginCache: MathPlugin | null = null
let mathPluginLoad: Promise<MathPlugin> | null = null

// Cover the delimiters/environments normalized by preprocessMarkdown, including
// partially streamed openers. Prose needs neither KaTeX nor its parse cache.
const hasMath = (text: string) => /\$|\\(?:\(|\[|begin\s*\{)|(?:`{3,}|~{3,})math\b/i.test(text)

export function useMathPlugin(text: string): MathPlugin | null {
  const needed = hasMath(text)
  const [plugin, setPlugin] = useState(mathPluginCache)

  useEffect(() => {
    if (plugin || !needed) {
      return
    }

    let cancelled = false

    mathPluginLoad ??= import('@/lib/katex-memo')
      .then(({ createMemoizedMathPlugin }) => {
        mathPluginCache ??= createMemoizedMathPlugin({ singleDollarTextMath: true })

        return mathPluginCache
      })
      .catch(error => {
        mathPluginLoad = null
        throw error
      })

    void mathPluginLoad
      .then(math => {
        if (!cancelled) {
          setPlugin(math)
        }
      })
      .catch(() => undefined) // Keep readable source text if the optional chunk fails.

    return () => {
      cancelled = true
    }
  }, [plugin, needed])

  return plugin
}
