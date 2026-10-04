import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, expect, it, vi } from 'vitest'

import { LookCard } from '@/components/onboarding-chat/cards/setup'
import { $onboardingAnswers, DEFAULT_ANSWERS } from '@/store/onboarding-answers'

const setMode = vi.fn()
vi.mock('@/themes', () => ({ useTheme: () => ({ mode: 'light', setMode }) }))
vi.mock('@/app/chat/composer/focus', () => ({ requestComposerSubmit: vi.fn(() => true) }))

afterEach(() => {
  cleanup()
  $onboardingAnswers.set({ ...DEFAULT_ANSWERS, committed: [] })
  vi.clearAllMocks()
})

it('offers only white and black during onboarding', () => {
  render(<LookCard attrs={{}} locked={false} />)
  expect(screen.getAllByRole('button', { pressed: false }).map(button => button.textContent)).toContain('Black')
  fireEvent.click(screen.getByRole('button', { name: 'Black' }))
  expect(setMode).toHaveBeenCalledWith('dark')
  expect(screen.queryByLabelText('Custom color')).toBeNull()
})
