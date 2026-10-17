import { BRAND_MARKS } from '@/components/ui/brand-logos'
import { cn } from '@/lib/utils'

/** Which brand mark stands for a provider id the app knows (OAuth ids, key ids, engine slugs). */
const PROVIDER_MARK: Record<string, string> = {
  anthropic: 'anthropic',
  deepseek: 'deepseek',
  fireworks: 'fireworks',
  gemini: 'gemini',
  google: 'gemini',
  'google-gemini': 'gemini',
  huggingface: 'huggingface',
  kimi: 'kimi',
  'kimi-coding': 'kimi',
  meta: 'meta',
  minimax: 'minimax',
  'minimax-oauth': 'minimax',
  mistral: 'mistral',
  nvidia: 'nvidia',
  ollama: 'ollama',
  openai: 'openai',
  'openai-codex': 'openai',
  openrouter: 'openrouter',
  qwen: 'qwen',
  'qwen-oauth': 'qwen',
  xai: 'xai',
  'xai-oauth': 'xai',
  zai: 'zhipu',
  zhipu: 'zhipu'
}

export const providerMarkId = (id: string | undefined | null): null | string => PROVIDER_MARK[(id ?? '').toLowerCase()] ?? null

/**
 * A bare brand mark: no tile, no ring, no tint behind it. Colour is the surrounding text colour, so it reads
 * white on dark and ink on light. Renders nothing for an id with no real mark, rather than a made-up one.
 */
export function BrandLogo({ className, id }: { className?: string; id: null | string | undefined }) {
  const mark = id ? BRAND_MARKS[id] : undefined

  if (!mark) {
    return null
  }

  return (
    <svg
      aria-hidden
      className={cn('size-5 shrink-0', className)}
      fill="currentColor"
      fillRule={mark.evenodd ? 'evenodd' : 'nonzero'}
      focusable="false"
      viewBox="0 0 24 24"
    >
      {mark.paths.map(d => (
        <path d={d} key={d} />
      ))}
    </svg>
  )
}
