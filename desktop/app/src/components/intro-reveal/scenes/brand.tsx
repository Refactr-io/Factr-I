import type { ComponentProps } from 'react'

import { HelmetMark } from '@/components/helmet-mark'

interface BrandCloseProps extends ComponentProps<'div'> {}

export function BrandClose({ ref }: BrandCloseProps) {
  return (
    <div
      className="pointer-events-none absolute inset-0 flex flex-col items-center justify-center gap-[3.2vmin] opacity-0"
      ref={ref}
      style={{ willChange: 'transform, opacity' }}
    >
      <HelmetMark className="h-[32vmin] w-auto" tone="dark" />
      <div className="flex flex-col items-center gap-[1.6vmin]">
        <h1
          className="text-[10.2vmin] leading-none uppercase text-white/95"
          style={{
            fontFamily: "'Collapse', sans-serif",
            fontWeight: 700,
            letterSpacing: '0.06em',
            textShadow: '0 2px 24px rgba(0,0,0,0.45)'
          }}
        >
          Factr-I
        </h1>
        <p
          className="text-[2vmin] uppercase tracking-[0.42em] text-white/50"
          style={{ fontFamily: "'Collapse', sans-serif" }}
        >
          Your agent, everywhere
        </p>
      </div>
    </div>
  )
}
