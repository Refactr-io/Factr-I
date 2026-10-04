import { cva, type VariantProps } from 'class-variance-authority'
import { Switch as SwitchPrimitive } from 'radix-ui'
import * as React from 'react'

import { cn } from '@/lib/utils'

const switchVariants = cva(
  'peer inline-flex shrink-0 items-center rounded-full bg-[color-mix(in_srgb,var(--dt-foreground)_16%,transparent)] p-0.5 transition-colors duration-200 ease-out outline-none focus-visible:ring-[0.1875rem] focus-visible:ring-ring/50 disabled:cursor-not-allowed disabled:opacity-50 data-[state=checked]:bg-primary disabled:data-[state=checked]:bg-(--ui-text-tertiary) motion-reduce:transition-none',
  {
    variants: {
      size: {
        default: 'h-5 w-9',
        xs: 'h-4 w-7'
      }
    },
    defaultVariants: {
      size: 'default'
    }
  }
)

// The thumb stays white in both states, so it reads against the dim off track and the dark on track alike.
const switchThumbVariants = cva(
  'pointer-events-none block rounded-full bg-white shadow-[0_0.0625rem_0.125rem_rgb(0_0_0/0.28)] ring-0 transition-transform duration-200 ease-[cubic-bezier(0.3,1.25,0.5,1)] data-[state=unchecked]:translate-x-0 motion-reduce:transition-none',
  {
    variants: {
      size: {
        default: 'size-4 data-[state=checked]:translate-x-4',
        xs: 'size-3 data-[state=checked]:translate-x-3'
      }
    },
    defaultVariants: {
      size: 'default'
    }
  }
)

function Switch({
  className,
  size,
  ...props
}: React.ComponentProps<typeof SwitchPrimitive.Root> & VariantProps<typeof switchVariants>) {
  return (
    <SwitchPrimitive.Root className={cn(switchVariants({ size }), className)} data-slot="switch" {...props}>
      <SwitchPrimitive.Thumb className={switchThumbVariants({ size })} data-slot="switch-thumb" />
    </SwitchPrimitive.Root>
  )
}

export { Switch }
