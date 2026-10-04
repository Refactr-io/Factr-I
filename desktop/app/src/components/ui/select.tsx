import './select-glide.css'

import { Select as SelectPrimitive } from 'radix-ui'
import * as React from 'react'

import { Codicon } from '@/components/ui/codicon'
import { type ControlVariantProps, controlVariants } from '@/components/ui/control'
import { usePopoverPortalContainer } from '@/components/ui/dialog-portal-context'
import { useFieldLabelId } from '@/components/ui/field-label'
import { cn } from '@/lib/utils'

function Select({ ...props }: React.ComponentProps<typeof SelectPrimitive.Root>) {
  return <SelectPrimitive.Root data-slot="select" {...props} />
}

function SelectTrigger({
  className,
  children,
  size,
  ...props
}: React.ComponentProps<typeof SelectPrimitive.Trigger> & ControlVariantProps) {
  const labelId = useFieldLabelId()

  return (
    <SelectPrimitive.Trigger
      aria-labelledby={props['aria-label'] ? undefined : labelId}
      className={cn(
        controlVariants({ size }),
        'group/select flex items-center justify-between gap-2 whitespace-nowrap data-placeholder:text-muted-foreground [&_svg]:pointer-events-none [&_svg]:shrink-0',
        className
      )}
      data-slot="select-trigger"
      {...props}
    >
      {children}
      <SelectPrimitive.Icon asChild>
        <Codicon
          className="opacity-60 transition-transform duration-200 ease-out group-data-[state=open]/select:rotate-180 motion-reduce:transition-none"
          name="chevron-down"
          size="1rem"
        />
      </SelectPrimitive.Icon>
    </SelectPrimitive.Trigger>
  )
}

function SelectValue({ ...props }: React.ComponentProps<typeof SelectPrimitive.Value>) {
  return <SelectPrimitive.Value data-slot="select-value" {...props} />
}

// Content only mounts while open, so this runs (and observes) once per open list.
// The pill follows whichever row Radix marks `data-highlighted`. Attached from
// the viewport's own ref callback: Radix mounts the list only while it is open,
// so an effect on SelectContent (which stays mounted) would run once, find no
// list, and never attach. One observer per open list, dropped on close.
function useGlidingHighlight(): React.RefCallback<HTMLDivElement> {
  return React.useCallback((viewport: HTMLDivElement | null) => {
    const pill = viewport?.querySelector<HTMLSpanElement>('.select-glide-pill')

    if (!viewport || !pill) {
      return
    }

    let frame = 0

    const sync = () => {
      const row = viewport.querySelector<HTMLElement>('[data-slot="select-item"][data-highlighted]')

      if (!row) {
        pill.removeAttribute('data-visible')

        return
      }

      pill.style.width = `${row.offsetWidth}px`
      pill.style.height = `${row.offsetHeight}px`
      pill.style.transform = `translate(${row.offsetLeft}px, ${row.offsetTop}px)`
      pill.setAttribute('data-visible', '')

      // Enable the glide only after the first placement so it opens on the selected row.
      if (!frame && !pill.hasAttribute('data-glide')) {
        frame = requestAnimationFrame(() => pill.setAttribute('data-glide', ''))
      }
    }

    sync()

    const observer = new MutationObserver(sync)

    observer.observe(viewport, { attributes: true, attributeFilter: ['data-highlighted'], subtree: true })

    // React 19: a ref callback may return its cleanup.
    return () => {
      observer.disconnect()
      cancelAnimationFrame(frame)
    }
  }, [])
}

function SelectContent({
  className,
  children,
  position = 'popper',
  ...props
}: React.ComponentProps<typeof SelectPrimitive.Content>) {
  // Portal into the enclosing dialog (if any) so the dropdown is a DOM
  // descendant of the dialog — keeps focus inside and stops the dialog closing
  // when the dropdown is dismissed. Falls back to document.body outside a dialog.
  const container = usePopoverPortalContainer()
  const viewportRef = useGlidingHighlight()

  return (
    <SelectPrimitive.Portal container={container}>
      <SelectPrimitive.Content
        className={cn(
          'select-glide-content menu-glass relative z-(--z-modal-popover) max-h-72 min-w-32 overflow-hidden rounded-md text-popover-foreground',
          position === 'popper' &&
            'data-[side=bottom]:translate-y-1 data-[side=left]:-translate-x-1 data-[side=right]:translate-x-1 data-[side=top]:-translate-y-1',
          className
        )}
        data-slot="select-content"
        position={position}
        {...props}
      >
        <SelectPrimitive.Viewport
          className={cn(
            'p-1',
            position === 'popper' && 'h-(--radix-select-trigger-height) w-full min-w-(--radix-select-trigger-width)'
          )}
          ref={viewportRef}
        >
          <span aria-hidden className="select-glide-pill rounded-sm" />
          {children}
        </SelectPrimitive.Viewport>
      </SelectPrimitive.Content>
    </SelectPrimitive.Portal>
  )
}

function SelectGroup({ ...props }: React.ComponentProps<typeof SelectPrimitive.Group>) {
  return <SelectPrimitive.Group data-slot="select-group" {...props} />
}

function SelectLabel({ className, ...props }: React.ComponentProps<typeof SelectPrimitive.Label>) {
  return (
    <SelectPrimitive.Label
      className={cn('px-2 py-1.5 text-[0.625rem] font-medium uppercase tracking-wide text-muted-foreground', className)}
      data-slot="select-label"
      {...props}
    />
  )
}

function SelectItem({ className, children, ...props }: React.ComponentProps<typeof SelectPrimitive.Item>) {
  return (
    <SelectPrimitive.Item
      className={cn(
        'relative flex w-full cursor-pointer items-center gap-2 rounded-sm py-1.5 pr-8 pl-2 text-xs outline-none select-none data-highlighted:text-accent-foreground data-highlighted:bg-transparent data-disabled:pointer-events-none data-disabled:cursor-default data-disabled:opacity-50',
        className
      )}
      data-slot="select-item"
      {...props}
    >
      <span className="absolute right-2 flex size-3.5 items-center justify-center">
        <SelectPrimitive.ItemIndicator>
          <Codicon name="check" size="1rem" />
        </SelectPrimitive.ItemIndicator>
      </span>
      <SelectPrimitive.ItemText>{children}</SelectPrimitive.ItemText>
    </SelectPrimitive.Item>
  )
}

export { Select, SelectContent, SelectGroup, SelectItem, SelectLabel, SelectTrigger, SelectValue }
