// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { ConfigField, parseBoundedInteger } from './config-field'

afterEach(cleanup)

const bounds = { max: 10_000_000, min: 0 }

describe('parseBoundedInteger', () => {
  it('accepts whole numbers in range and empty (0 = detected)', () => {
    expect(parseBoundedInteger('8192', bounds)).toBe(8192)
    expect(parseBoundedInteger('', bounds)).toBe(0)
    expect(parseBoundedInteger('10000000', bounds)).toBe(10_000_000)
  })

  it('rejects negatives, text, decimals and huge values', () => {
    for (const bad of ['-5', 'abc', '1.5', '99999999999999', '1e3']) {
      expect(parseBoundedInteger(bad, bounds)).toBeNull()
    }
  })
})

describe('context window field', () => {
  const setup = () => {
    const onChange = vi.fn()

    render(
      <ConfigField
        onChange={onChange}
        schema={{ type: 'number' } as never}
        schemaKey="model_context_length"
        value={8192}
      />
    )

    return { input: screen.getByRole('textbox') as HTMLInputElement, onChange }
  }

  it('shows an inline error and never saves an invalid value', () => {
    const { input, onChange } = setup()

    for (const bad of ['-5', 'abc', '1.5', '99999999999999']) {
      fireEvent.change(input, { target: { value: bad } })
      expect(screen.getByRole('alert').textContent).toContain('whole number')
    }

    expect(onChange).not.toHaveBeenCalled()
  })

  it('saves a valid value and clears the error', () => {
    const { input, onChange } = setup()

    fireEvent.change(input, { target: { value: 'abc' } })
    fireEvent.change(input, { target: { value: '16000' } })

    expect(onChange).toHaveBeenCalledWith(16000)
    expect(screen.queryByRole('alert')).toBeNull()
  })
})
