// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, expect, test, vi } from 'vitest'

import { en } from '@/i18n/en'

const requestGateway = vi.hoisted(() => vi.fn())

vi.mock('@/app/gateway/hooks/use-gateway-request', () => ({ useGatewayRequest: () => ({ requestGateway }) }))
vi.mock('@/store/notifications', () => ({ notifyError: vi.fn() }))

import { AutoLearningSetting } from './auto-learning-setting'

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

test('binds the switch to the engine learning.enabled setting', async () => {
  requestGateway.mockImplementation(async (method: string) =>
    method === 'config.get' ? { value: true } : { value: false }
  )
  render(<AutoLearningSetting />)

  const toggle = await screen.findByRole('switch', { name: en.settings.config.autoLearningTitle })
  await waitFor(() => expect(toggle.getAttribute('aria-checked')).toBe('true'))
  expect(requestGateway).toHaveBeenCalledWith('config.get', { key: 'learning.enabled' })

  fireEvent.click(toggle)
  await waitFor(() =>
    expect(requestGateway).toHaveBeenCalledWith('config.set', { key: 'learning.enabled', value: false })
  )
  expect(toggle.getAttribute('aria-checked')).toBe('false')
})
