import { atom } from 'nanostores'

import { storedStringRecord } from '@/lib/storage'

const STORAGE_KEY = 'factr.desktop.profilePictures'
export const PROFILE_PICTURE_MAX_BYTES = 256 * 1024
const TYPES = ['image/png', 'image/jpeg', 'image/webp']

export const $profilePictures = atom<Record<string, string>>(storedStringRecord(STORAGE_KEY))

export const profilePictureKey = (connectionId: null | string, name: string) =>
  JSON.stringify([connectionId || 'local', name || 'default'])

export function validProfilePicture(value: string | undefined): string | undefined {
  return value && value.length <= Math.ceil(PROFILE_PICTURE_MAX_BYTES / 3) * 4 + 32 &&
    /^data:image\/(?:png|jpeg|webp);base64,[A-Za-z0-9+/]+={0,2}$/.test(value) ? value : undefined
}

// shortcut: pictures are device-local presentation metadata, extend profile export only if requested.
export function setProfilePicture(key: string, picture: null | string): void {
  if (picture !== null && !validProfilePicture(picture)) {
    throw new Error('Invalid profile picture')
  }
  const next = { ...$profilePictures.get() }
  if (picture) {
    next[key] = picture
  } else {
    delete next[key]
  }
  // Persist first: quota failures must leave the existing picture intact.
  localStorage.setItem(STORAGE_KEY, JSON.stringify(next))
  $profilePictures.set(next)
}

export function moveProfilePicture(connectionId: null | string, from: string, to: string): void {
  const oldKey = profilePictureKey(connectionId, from)
  const picture = validProfilePicture($profilePictures.get()[oldKey])
  if (picture) {
    const next = { ...$profilePictures.get(), [profilePictureKey(connectionId, to)]: picture }
    delete next[oldKey]
    localStorage.setItem(STORAGE_KEY, JSON.stringify(next))
    $profilePictures.set(next)
  }
}

export async function readProfilePicture(file: File): Promise<string> {
  if (!TYPES.includes(file.type) || !file.size || file.size > PROFILE_PICTURE_MAX_BYTES) {
    throw new Error('Invalid profile picture type or size')
  }
  const image = await createImageBitmap(file)
  try {
    if (image.width > 4096 || image.height > 4096) {
      throw new Error('Profile picture dimensions exceed 4096px')
    }
  } finally {
    image.close()
  }
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    reader.onerror = () => reject(new Error('Could not read profile picture'))
    reader.onload = () => {
      const result = validProfilePicture(typeof reader.result === 'string' ? reader.result : undefined)
      if (result) { resolve(result) } else { reject(new Error('Invalid profile picture')) }
    }
    reader.readAsDataURL(file)
  })
}
