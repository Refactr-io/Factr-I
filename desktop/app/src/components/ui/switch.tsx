import { cva, type VariantProps } from 'class-variance-authority'
import { Switch as SwitchPrimitive } from 'radix-ui'
import * as React from 'react'

import { cn } from '@/lib/utils'

const switchVariants = cva(
  'peer inline-flex shrink-0 items-center rounded-full bg-[color-mix(in_srgb,var(--dt-foreground)_16%,transparent)] p-0.5 transition-[background-color,transform] duration-[180ms] ease-[cubic-bezier(0.23,1,0.32,1)] outline-none active:scale-[0.96] focus-visible:ring-[0.1875rem] focus-visible:ring-ring/50 disabled:cursor-not-allowed disabled:opacity-50 data-[state=checked]:bg-(--ui-control-on) disabled:data-[state=checked]:bg-(--ui-text-tertiary) motion-reduce:transition-none motion-reduce:active:scale-100',
  {
    variants: {
      size: {
        default: 'h-5 w-8',
        xs: 'h-4 w-7'
      }
    },
    defaultVariants: {
      size: 'default'
    }
  }
)

// The thumb is Rose Alloy in both states, so it reads against the dim off track and the green on track alike. No shadow.
const switchThumbVariants = cva(
  'pointer-events-none block rounded-full bg-(--ui-control-on-foreground) ring-0 transition-transform duration-[180ms] ease-[cubic-bezier(0.23,1,0.32,1)] data-[state=unchecked]:translate-x-0 motion-reduce:transition-none',
  {
    variants: {
      size: {
        default: 'size-4 data-[state=checked]:translate-x-3',
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
    <SwitchPrimitive.Root
      {...props}
      className={cn(switchVariants({ size }), className)}
      data-slot="switch"
      onKeyDown={event => {
        props.onKeyDown?.(event)

        // Space is the switch pattern; Enter does the same so a keyboard user is never left guessing.
        if (event.key === 'Enter' && !event.defaultPrevented) {
          event.preventDefault()
          event.currentTarget.click()
        }
      }}
    >
      <SwitchPrimitive.Thumb className={switchThumbVariants({ size })} data-slot="switch-thumb" />
    </SwitchPrimitive.Root>
  )
}

export { Switch }
