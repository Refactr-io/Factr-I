import { BRAND_MARKS } from '@/components/ui/brand-logos'
import { cn } from '@/lib/utils'

/** Which brand mark stands for a provider id the app knows (OAuth ids, key ids, engine slugs). */
const PROVIDER_MARK: Record<string, string> = {
  anthropic: 'anthropic',
  'claude-code': 'anthropic',
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

export const providerMarkId = (id: string | undefined | null): null | string =>
  PROVIDER_MARK[(id ?? '').toLowerCase()] ?? null

/**
 * A bare brand mark: no tile, no ring, no tint behind it. Full-colour marks keep their own fills. Single-colour
 * marks are painted in the brand colour, and fall back to the text colour (white on dark, ink on light) on the
 * theme where that colour would not read. Renders nothing for an id with no real mark, never a made-up one.
 */
export function BrandLogo({ className, id }: { className?: string; id: null | string | undefined }) {
  const mark = id ? BRAND_MARKS[id] : undefined

  if (!mark) {
    return null
  }

  const common = {
    'aria-hidden': true,
    className: cn('size-5 shrink-0', className),
    focusable: 'false',
    viewBox: mark.viewBox
  } as const

  if (mark.raw) {
    // Trusted static markup from the icon packages (gradients included); ids are prefixed per mark.
    return <svg {...common} dangerouslySetInnerHTML={{ __html: mark.raw }} />
  }

  const light = mark.colorLight === null ? 'currentColor' : (mark.color ?? 'currentColor')
  const dark = mark.colorDark === null ? 'currentColor' : (mark.color ?? 'currentColor')

  return (
    <svg
      {...common}
      className={cn(common.className, 'fill-(--brand-light) dark:fill-(--brand-dark)')}
      fillRule={mark.evenodd ? 'evenodd' : 'nonzero'}
      style={{ '--brand-dark': dark, '--brand-light': light } as React.CSSProperties}
    >
      {mark.paths?.map(d => (
        <path d={d} key={d} />
      ))}
    </svg>
  )
}
