// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { ConfigField, parseList } from './config-field'

afterEach(cleanup)

describe('parseList', () => {
  it('splits on commas, trims and drops empties, keeping spaces inside an item', () => {
    expect(parseList('git push, rm -rf ,, ^echo hello')).toEqual(['git push', 'rm -rf', '^echo hello'])
    expect(parseList('')).toEqual([])
  })
})

describe('list field', () => {
  it('keeps the spaces and commas you type while the field is focused', () => {
    const onChange = vi.fn()

    render(<ConfigField onChange={onChange} schema={{ type: 'list' } as never} schemaKey="approvals.always_ask" value={[]} />)

    const input = screen.getByRole('textbox') as HTMLInputElement

    fireEvent.change(input, { target: { value: 'git ' } })
    expect(input.value).toBe('git ')
    fireEvent.change(input, { target: { value: 'git push, ' } })
    expect(input.value).toBe('git push, ')
    expect(onChange).toHaveBeenLastCalledWith(['git push'])

    fireEvent.change(input, { target: { value: 'git push, rm -rf' } })
    expect(onChange).toHaveBeenLastCalledWith(['git push', 'rm -rf'])
  })
})
