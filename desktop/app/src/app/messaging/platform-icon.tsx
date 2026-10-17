import type { ComponentPropsWithoutRef } from 'react'
import { forwardRef, memo } from 'react'

import { BrandLogo } from '@/components/ui/brand-logo'
import { Globe, Link as LinkIcon, Mail, MessageSquareText } from '@/lib/icons'
import { cn } from '@/lib/utils'

// Bare marks, no tile and no tint (see BrandLogo). Where no real mark exists (Slack, DingTalk, Feishu / Lark,
// WeCom and Microsoft Teams are not in the icon sources, their owners had them removed) the row gets a plain
// neutral glyph instead of a letter in a coloured box or an invented logo.
const PLATFORM_MARKS: Record<string, string> = {
  bluebubbles: 'imessage',
  discord: 'discord',
  google_chat: 'googlechat',
  homeassistant: 'homeassistant',
  line: 'line',
  matrix: 'matrix',
  mattermost: 'mattermost',
  ntfy: 'ntfy',
  photon: 'photon',
  qqbot: 'qq',
  signal: 'signal',
  slack: 'slack',
  simplex: 'simplex',
  teams: 'teams',
  telegram: 'telegram',
  weixin: 'wechat',
  whatsapp: 'whatsapp',
  whatsapp_cloud: 'whatsapp',
  yuanbao: 'yuanbao'
}

const GENERIC_GLYPH: Record<string, typeof Globe> = {
  api_server: Globe,
  email: Mail,
  webhook: LinkIcon
}

interface PlatformAvatarProps extends Omit<ComponentPropsWithoutRef<'span'>, 'children'> {
  platformId: string
  platformName: string
}

// forwardRef + spreading ...rest is required so a wrapping <Tip> (Radix
// Tooltip's `asChild`) can actually attach its trigger: asChild clones this
// component and injects a ref plus pointer/focus/aria handlers onto it. A
// plain function component with no ref/rest forwarding drops all of that
// silently — the tooltip renders but never opens (#67500).
export const PlatformAvatar = memo(
  forwardRef<HTMLSpanElement, PlatformAvatarProps>(function PlatformAvatar(
    { className, platformId, platformName, ...rest },
    ref
  ) {
    const mark = PLATFORM_MARKS[platformId]
    const Glyph = GENERIC_GLYPH[platformId] ?? MessageSquareText

    return (
      <span
        aria-hidden="true"
        className={cn('relative inline-grid size-6 shrink-0 place-items-center text-(--ui-text-secondary)', className)}
        data-platform={platformName}
        ref={ref}
        {...rest}
      >
        {mark ? (
          <BrandLogo className="size-[68%]" id={mark} />
        ) : (
          <Glyph className="size-[68%] text-(--ui-text-tertiary)" />
        )}
      </span>
    )
  })
)
