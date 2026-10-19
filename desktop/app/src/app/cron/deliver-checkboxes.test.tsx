import { cleanup, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { Field } from '@/components/ui/field'
import { en } from '@/i18n/en'

import { DeliverCheckboxes } from './index'

const hidden = vi.hoisted(() => ({ bots: false }))

vi.mock('@/lib/hidden-features', () => ({
  HIDDEN_BUNDLED_PLUGINS: new Set(),
  get botsHidden() {
    return hidden.bots
  }
}))

afterEach(cleanup)

// Regression for the delivery-target group losing its accessible name: `Field`
// renders the visible label as `<label htmlFor>`, which only associates with
// labelable elements (input, select, ...) — not a `div[role="group"]`. The
// group must pick up the label via `aria-labelledby` instead.
describe('DeliverCheckboxes accessible name', () => {
  it('exposes the Field label as the group name', () => {
    render(
      <Field htmlFor="cron-deliver" label={en.cron.deliverLabel}>
        <DeliverCheckboxes
          c={en.cron}
          id="cron-deliver"
          onChange={() => {}}
          targets={[{ home_env_var: null, home_target_set: true, id: 'local', name: 'This desktop' }]}
          value="local"
        />
      </Field>
    )

    expect(screen.getByRole('group', { name: en.cron.deliverLabel })).toBeTruthy()
  })
})

describe('DeliverCheckboxes bot targets', () => {
  const targets = [
    { home_env_var: null, home_target_set: true, id: 'local', name: 'This computer' },
    { home_env_var: null, home_target_set: true, id: 'bot-chat:default', name: 'Bot Chat (default)' }
  ]

  const renderTargets = () =>
    render(<DeliverCheckboxes c={en.cron} id="cron-deliver" onChange={() => {}} targets={targets} value="local" />)

  it('does not offer Bot Chat targets while bots are hidden', () => {
    hidden.bots = true
    renderTargets()

    expect(screen.queryByText(/Bot Chat/)).toBeNull()
  })

  it('offers Bot Chat targets when bots are shown', () => {
    hidden.bots = false
    renderTargets()

    expect(screen.getByText(/Bot Chat/)).toBeTruthy()
  })
})
