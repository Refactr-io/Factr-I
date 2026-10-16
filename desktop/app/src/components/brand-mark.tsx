import { HelmetMark } from '@/components/helmet-mark'
import { cn } from '@/lib/utils'

// Brand badge: the bare Factr-I helmet, identical wherever it appears (about, updates, setup). Size via className
// (default size-14); the mark is taller than wide, so it fits the box by height.
export function BrandMark({ className, ...props }: React.ComponentProps<'span'>) {
  return (
    <span className={cn('inline-flex size-14 shrink-0 items-center justify-center', className)} {...props}>
      <HelmetMark className="size-full" />
    </span>
  )
}
