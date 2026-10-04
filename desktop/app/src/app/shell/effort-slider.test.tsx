import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { en } from '@/i18n/en'

import { EffortSlider, supportedEfforts } from './effort-slider'

afterEach(cleanup)

const copy = en.shell.modelOptions

describe('EffortSlider', () => {
  it('ArrowRight commits the next level and the thumb announces its label', () => {
    const onCommit = vi.fn()

    render(<EffortSlider copy={copy} onCommit={onCommit} value="medium" />)

    const thumb = screen.getByRole('slider', { name: 'Effort' })

    expect(thumb.getAttribute('aria-valuetext')).toBe('Medium')

    fireEvent.keyDown(thumb, { key: 'ArrowRight' })

    expect(onCommit).toHaveBeenCalledWith('high')
    // The local preview moves with the key even before the owner re-renders.
    expect(thumb.getAttribute('aria-valuetext')).toBe('High')
  })

  it('keeps slider keys from reaching the owning menu', () => {
    const onMenuKey = vi.fn()

    render(
      <div onKeyDown={onMenuKey}>
        <EffortSlider copy={copy} onCommit={vi.fn()} value="low" />
      </div>
    )

    fireEvent.keyDown(screen.getByRole('slider'), { key: 'ArrowLeft' })

    expect(onMenuKey).not.toHaveBeenCalled()
  })
})

describe('supportedEfforts + levels', () => {
  const luna = ['low', 'medium', 'high', 'xhigh', 'max']

  it('keeps ladder order and drops unknown or unsupported levels', () => {
    expect(supportedEfforts(['max', 'low', 'bogus', 'medium'])).toEqual(['low', 'medium', 'max'])
    expect(supportedEfforts(undefined)).toBeUndefined()
    expect(supportedEfforts([])).toBeUndefined()
  })

  it("offers only the model's levels and never commits one it lacks", () => {
    const onCommit = vi.fn()

    render(<EffortSlider copy={copy} levels={supportedEfforts(luna)} onCommit={onCommit} value="max" />)

    const slider = screen.getByRole('slider', { name: 'Effort' })

    expect(slider.getAttribute('aria-valuemax')).toBe('4')

    fireEvent.keyDown(slider, { key: 'ArrowRight' })
    expect(onCommit).not.toHaveBeenCalled()

    fireEvent.keyDown(slider, { key: 'ArrowLeft' })
    expect(onCommit).toHaveBeenCalledWith('xhigh')
  })

  it('snaps a level the model lacks to the nearest one below', () => {
    render(<EffortSlider copy={copy} levels={supportedEfforts(luna)} onCommit={vi.fn()} value="ultra" />)

    expect(screen.getByRole('slider', { name: 'Effort' }).getAttribute('aria-valuetext')).toBe('Max')
  })
})
