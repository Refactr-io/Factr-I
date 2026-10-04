import type { Icon } from '@tabler/icons-react'
import type * as React from 'react'

import {
  ChatCircle,
  ClockCountdown,
  DotsThree,
  GearSix,
  Globe as GlobeGlyph,
  House,
  Microphone,
  NotePencil,
  SidebarSimple,
  X
} from '@/components/icons/phosphor'
import { cn } from '@/lib/utils'

// Codicon names drawn with Phosphor instead (one line family for the chrome;
// picked with aria-icons). Everything else stays a codicon font glyph.
const PHOSPHOR: Record<string, { Glyph: React.ComponentType<React.SVGProps<SVGSVGElement>>; mirror?: boolean }> = {
  // Not a codicon: the Phosphor globe, drawn the same everywhere the browser appears (tool rows, palette).
  'browser-window': { Glyph: GlobeGlyph },
  close: { Glyph: X },
  comment: { Glyph: ChatCircle },
  ellipsis: { Glyph: DotsThree },
  home: { Glyph: House },
  'layout-sidebar-left': { Glyph: SidebarSimple },
  'layout-sidebar-right': { Glyph: SidebarSimple, mirror: true },
  mic: { Glyph: Microphone },
  // Not a codicon: the sidebar's compose button (was `robot`, which Bots keeps).
  'new-session': { Glyph: NotePencil },
  'settings-gear': { Glyph: GearSix },
  watch: { Glyph: ClockCountdown }
}

export interface CodiconProps extends React.HTMLAttributes<HTMLElement> {
  name: string
  size?: number | string
  spinning?: boolean
}

export function Codicon({ className, name, size, spinning, style, ...props }: CodiconProps) {
  const swap = PHOSPHOR[name]

  if (swap) {
    const { Glyph, mirror } = swap

    return (
      <i
        aria-hidden="true"
        className={cn('codicon-svg', spinning && 'codicon-modifier-spin', className)}
        // The font's `codicon-<name>` class would draw its glyph too; name it here instead.
        data-codicon={name}
        style={{ ...(size != null && size !== '' ? { fontSize: size } : {}), ...style }}
        {...props}
      >
        <Glyph style={mirror ? { transform: 'scaleX(-1)' } : undefined} />
      </i>
    )
  }

  return (
    <i
      aria-hidden="true"
      className={cn('codicon', `codicon-${name}`, spinning && 'codicon-modifier-spin', className)}
      style={{ ...(size != null && size !== '' ? { fontSize: size } : {}), ...style }}
      {...props}
    />
  )
}

/** Wrap a codicon as a Tabler-shaped icon for nav rows that expect `IconComponent`. */
export function codiconIcon(name: string): Icon {
  function CodiconIcon({ className }: { className?: string }) {
    return <Codicon aria-hidden className={cn('leading-none', className)} name={name} size="1em" />
  }

  CodiconIcon.displayName = `Codicon(${name})`

  return CodiconIcon as Icon
}
