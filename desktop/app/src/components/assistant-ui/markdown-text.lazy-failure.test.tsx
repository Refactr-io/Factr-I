import { cleanup, render, waitFor } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { MarkdownTextContent } from './markdown-text'

const { codeLoad, mathLoad } = vi.hoisted(() => ({ codeLoad: vi.fn(), mathLoad: vi.fn() }))
vi.mock('@streamdown/code', () => {
  codeLoad()
  throw new Error('fixture: code chunk unavailable')
})
vi.mock('@/lib/katex-memo', () => {
  mathLoad()
  throw new Error('fixture: math chunk unavailable')
})

afterEach(cleanup)

describe('optional Markdown chunk failures', () => {
  it('keeps fenced code visible if the highlighting plugin fails to load', async () => {
    const { container } = render(<MarkdownTextContent isRunning={false} text={'```text\nReadable code fallback\n```'} />)

    await waitFor(() => expect(codeLoad).toHaveBeenCalled())
    expect(container.textContent).toContain('Readable code fallback')
  })

  it('keeps math source visible if the math plugin fails to load', async () => {
    const { container } = render(<MarkdownTextContent isRunning={false} text="The formula $x+y$ remains readable." />)

    await waitFor(() => expect(mathLoad).toHaveBeenCalled())
    expect(container.textContent).toContain('x+y')
    expect(container.textContent).toContain('remains readable')
  })
})
