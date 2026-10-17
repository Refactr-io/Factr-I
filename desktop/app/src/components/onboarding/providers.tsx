import { BrandLogo, providerMarkId } from '@/components/ui/brand-logo'
import { RowButton } from '@/components/ui/row-button'
import { useI18n } from '@/i18n'
import { Check, ChevronRight, KeyRound, Monitor, Terminal } from '@/lib/icons'
import { PROVIDER_DISPLAY_NAMES } from '@/lib/model-status-label'
import type { OAuthProvider } from '@/types/factr'

// Titles live in PROVIDER_DISPLAY_NAMES (shared with the model pill); this is
// only the featured order. Both Anthropic entries sit at the bottom: the API-key
// path first, then the subscription OAuth path (only works with extra usage credits).
const PROVIDER_ORDER: Record<string, number> = {
  'openai-codex': 0,
  'minimax-oauth': 1,
  'qwen-oauth': 2,
  'xai-oauth': 3,
  anthropic: 4,
  'claude-code': 5
}

export const providerTitle = (p: OAuthProvider) => PROVIDER_DISPLAY_NAMES[p.id] ?? p.name
const orderOf = (p: OAuthProvider) => PROVIDER_ORDER[p.id] ?? 99

export const sortProviders = (providers: OAuthProvider[]) =>
  [...providers].sort((a, b) => orderOf(a) - orderOf(b) || a.name.localeCompare(b.name))

/** The 20px lead slot every provider row shares: the real mark when there is one, a quiet glyph when not. */
function RowLead({ children }: { children: React.ReactNode }) {
  return <span className="grid size-5 shrink-0 place-items-center text-(--ui-text-secondary)">{children}</span>
}

// The suggested option is the one filled row on the screen: Spring Verdant with Blackthorn Ink text (12.2:1;
// white on this green would be 1.5:1 and fail). Everything else stays a neutral tone. First position and the
// fill carry the recommendation; the word "Recommended" is plain text, not a tag.
export function FeaturedProviderRow({
  onSelect,
  provider
}: {
  onSelect: (provider: OAuthProvider) => void
  provider: OAuthProvider
}) {
  const { t } = useI18n()
  const loggedIn = provider.status?.logged_in

  return (
    <button
      className="group relative flex w-full items-center gap-3 rounded-lg bg-[#67EAA2] px-3 py-2.5 text-left text-[#151313] transition-colors hover:bg-[#5CDB96]"
      onClick={() => onSelect(provider)}
      type="button"
    >
      <span className="grid size-5 shrink-0 place-items-center">
        <BrandLogo id={providerMarkId(provider.id)} />
      </span>
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="text-[length:var(--conversation-text-font-size)] font-medium">
            {providerTitle(provider)}
          </span>
          {loggedIn ? (
            <span className="inline-flex items-center gap-1 text-xs text-[#151313]/70">
              <Check className="size-3" />
              {t.onboarding.connected}
            </span>
          ) : (
            <span className="text-xs text-[#151313]/80">{t.onboarding.recommended}</span>
          )}
        </div>
        <p className="mt-0.5 text-xs leading-5 text-[#151313]/72">{t.onboarding.featuredPitch}</p>
      </div>
      <ChevronRight className="size-4 shrink-0 text-[#151313]/70 transition group-hover:text-[#151313]" />
    </button>
  )
}

function ConnectedTag() {
  const { t } = useI18n()

  return (
    <span className="inline-flex items-center gap-1 text-xs text-(--ui-text-secondary)">
      <Check className="size-3" />
      {t.onboarding.connected}
    </span>
  )
}

const PROVIDER_ROW_CLASS =
  'group flex w-full items-center gap-3 rounded-md px-3 py-2.5 text-left transition-colors hover:bg-(--ui-control-hover-background)'

/** Quick-key row for API-key providers (Fireworks leads the expanded list, OpenRouter further down). */
export function KeyProviderRow({
  lead,
  onClick,
  pitch,
  title
}: {
  lead?: React.ReactNode
  onClick: () => void
  pitch: string
  title: string
}) {
  return (
    <RowButton className={PROVIDER_ROW_CLASS} onClick={onClick}>
      <RowLead>{lead ?? <KeyRound className="size-4" />}</RowLead>
      <div className="min-w-0 flex-1">
        <span className="text-[length:var(--conversation-text-font-size)] font-medium">{title}</span>
        <p className="mt-0.5 text-xs leading-5 text-muted-foreground">{pitch}</p>
      </div>
      <ChevronRight className="size-4 shrink-0 text-muted-foreground transition group-hover:text-foreground" />
    </RowButton>
  )
}

export function FireworksProviderRow({ onClick }: { onClick: () => void }) {
  const { t } = useI18n()

  return (
    <KeyProviderRow
      lead={<BrandLogo id="fireworks" />}
      onClick={onClick}
      pitch={t.onboarding.fireworksPitch}
      title="Fireworks AI"
    />
  )
}

/** Onboarding row for the managed local runtime: no account, no key — the
 *  destination is the Local Models pane where install/download live. */
export function LocalModelsProviderRow({ onClick }: { onClick: () => void }) {
  const { t } = useI18n()

  return (
    <KeyProviderRow
      lead={<Monitor className="size-4" />}
      onClick={onClick}
      pitch={t.onboarding.localModelsPitch}
      title={t.onboarding.localModelsTitle}
    />
  )
}

export function OpenRouterProviderRow({ onClick }: { onClick: () => void }) {
  const { t } = useI18n()

  return (
    <KeyProviderRow
      lead={<BrandLogo id="openrouter" />}
      onClick={onClick}
      pitch={t.onboarding.openRouterPitch}
      title="OpenRouter"
    />
  )
}

export function ProviderRow({
  onSelect,
  provider
}: {
  onSelect: (provider: OAuthProvider) => void
  provider: OAuthProvider
}) {
  const { t } = useI18n()
  const loggedIn = provider.status?.logged_in
  const Trail = provider.flow === 'external' ? Terminal : ChevronRight
  const mark = providerMarkId(provider.id)

  return (
    <RowButton className={PROVIDER_ROW_CLASS} onClick={() => onSelect(provider)}>
      <RowLead>{mark ? <BrandLogo id={mark} /> : <KeyRound className="size-4" />}</RowLead>
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="text-[length:var(--conversation-text-font-size)] font-medium">
            {providerTitle(provider)}
          </span>
          {loggedIn ? <ConnectedTag /> : null}
        </div>
        <p className="mt-0.5 text-xs leading-5 text-muted-foreground">{t.onboarding.flowSubtitles[provider.flow]}</p>
      </div>
      <Trail className="size-4 shrink-0 text-muted-foreground transition group-hover:text-foreground" />
    </RowButton>
  )
}
