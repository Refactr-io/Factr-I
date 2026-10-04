import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { SegmentedControl } from './segmented-control'

const options = [
  { id: 'a', label: 'Alpha' },
  { id: 'b', label: 'Beta' },
  { id: 'c', label: 'Gamma' }
] as const

afterEach(cleanup)

describe('SegmentedControl', () => {
  it('reflects value in aria-checked with a roving tabindex', () => {
    render(<SegmentedControl onChange={() => {}} options={options} value="b" />)

    const radios = screen.getAllByRole('radio')

    expect(radios.map(r => r.getAttribute('aria-checked'))).toEqual(['false', 'true', 'false'])
    expect(radios.map(r => r.tabIndex)).toEqual([-1, 0, -1])
  })

  it('calls onChange with the id when an option is clicked', () => {
    const onChange = vi.fn()

    render(<SegmentedControl onChange={onChange} options={options} value="a" />)
    fireEvent.click(screen.getByRole('radio', { name: 'Gamma' }))

    expect(onChange).toHaveBeenCalledWith('c')
  })

  it('moves selection with the arrow keys and Home/End', () => {
    const onChange = vi.fn()

    render(<SegmentedControl onChange={onChange} options={options} value="a" />)
    const first = screen.getByRole('radio', { name: 'Alpha' })

    fireEvent.keyDown(first, { key: 'ArrowRight' })
    expect(onChange).toHaveBeenLastCalledWith('b')

    fireEvent.keyDown(first, { key: 'End' })
    expect(onChange).toHaveBeenLastCalledWith('c')
  })

  it('ignores input when disabled', () => {
    const onChange = vi.fn()

    render(<SegmentedControl disabled onChange={onChange} options={options} value="a" />)
    fireEvent.click(screen.getByRole('radio', { name: 'Beta' }))

    expect(onChange).not.toHaveBeenCalled()
  })
})
