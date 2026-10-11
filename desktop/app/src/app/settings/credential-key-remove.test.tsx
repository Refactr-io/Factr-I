// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, expect, it, vi } from 'vitest'

import type { EnvVarInfo } from '@/types/factr'

import { KeyField } from './credential-key-ui'

afterEach(cleanup)

const rowProps = (onClear: (key: string) => void) =>
  ({ edits: {}, onClear, onSave: vi.fn(), revealed: {}, saving: null, setEdits: vi.fn() }) as never

const info = (is_set: boolean) =>
  ({ category: 'provider', description: '', is_password: true, is_set, redacted_value: 'sk-f...0000' }) as EnvVarInfo

it('a saved key shows a Remove button without first entering edit mode', () => {
  const onClear = vi.fn()
  render(<KeyField info={info(true)} rowProps={rowProps(onClear)} varKey="FIREWORKS_API_KEY" />)

  fireEvent.click(screen.getByRole('button', { name: 'Remove' }))
  expect(onClear).toHaveBeenCalledWith('FIREWORKS_API_KEY')
})

it('an unset key offers no Remove', () => {
  render(<KeyField info={info(false)} rowProps={rowProps(vi.fn())} varKey="FIREWORKS_API_KEY" />)
  expect(screen.queryByRole('button', { name: 'Remove' })).toBeNull()
})

it('humanizeEnvName title-cases long words and keeps short acronyms', async () => {
  const { humanizeEnvName } = await import('./credential-key-ui')
  expect(humanizeEnvName('A2A_HOME_CHANNEL')).toBe('A2A Home Channel')
  expect(humanizeEnvName('BRV')).toBe('BRV')
  expect(humanizeEnvName('FAL')).toBe('FAL')
})
