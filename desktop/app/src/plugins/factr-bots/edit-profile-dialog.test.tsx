import type * as FactrSdk from '@factr/plugin-sdk'
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const { request } = vi.hoisted(() => ({ request: vi.fn() }))
vi.mock('@factr/plugin-sdk', async importOriginal => {
  const sdk = await importOriginal<typeof FactrSdk>()
  return { ...sdk, host: { ...sdk.host, request } }
})
vi.mock('./avatar-picker', () => ({ AvatarPicker: () => null }))
const { EditProfileDialog } = await import('./edit-profile-dialog')
beforeEach(() => request.mockReset())
afterEach(cleanup)

describe('the stored profile description', () => {
  it('loads the description even when the roster row is empty', async () => {
    request.mockResolvedValue({ description: 'Research internal documents' })
    render(<EditProfileDialog bot={{ name: 'research', description: '' }} open onClose={() => {}} />)
    await waitFor(() => expect((screen.getByRole('textbox', { name: 'Description' }) as HTMLTextAreaElement).value).toBe('Research internal documents'))
    expect(request).toHaveBeenCalledWith('profiles.describe', { name: 'research' })
  })
  it('keeps typing that happens before a slow description read finishes', async () => {
    let resolve!: (value: unknown) => void
    request.mockReturnValue(new Promise(r => { resolve = r }))
    render(<EditProfileDialog bot={{ name: 'research', description: '' }} open onClose={() => {}} />)
    const field = screen.getByRole('textbox', { name: 'Description' }) as HTMLTextAreaElement
    fireEvent.change(field, { target: { value: 'My edited role' } })
    await act(async () => resolve({ description: 'Old saved role' }))
    expect(field.value).toBe('My edited role')
  })
})
