import { useStore } from '@nanostores/react'

import { profileShortLabel } from '@/lib/profile-short-label'
import { cn } from '@/lib/utils'
import { $profilePictures, profilePictureKey, validProfilePicture } from '@/store/profile-pictures'

export function ProfilePicture({ connectionId, name, tone, className }: {
  connectionId: null | string
  name: string
  tone: null | string
  className?: string
}) {
  const pictures = useStore($profilePictures)
  const picture = validProfilePicture(pictures[profilePictureKey(connectionId, name)])
  return (
    <span aria-hidden className={cn('relative grid size-6 shrink-0 place-items-center overflow-hidden rounded-sm bg-(--ui-control-active-background) text-xs font-medium leading-none text-foreground', className)}
      style={tone ? { backgroundColor: `color-mix(in srgb, ${tone} 22%, var(--ui-bg-elevated))`, cornerShape: 'superellipse(1.3)' } : { cornerShape: 'superellipse(1.3)' }}>
      {profileShortLabel(name)}
      {picture && <img alt="" className="absolute inset-0 size-full object-cover" key={picture} onError={event => { event.currentTarget.style.visibility = 'hidden' }} src={picture} />}
    </span>
  )
}
