import { cleanup, render, waitFor } from '@testing-library/react'
import { Streamdown } from 'streamdown'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { MarkdownPreview } from '@/app/chat/right-rail/preview-file'
import { preprocessMarkdown } from '@/lib/markdown-preprocess'

import { MarkdownTextContent } from './markdown-text'

const { loaded } = vi.hoisted(() => ({ loaded: vi.fn() }))
vi.mock('@/lib/katex-memo', async importOriginal => {
  loaded()

  return importOriginal()
})

afterEach(cleanup)

describe('lazy math in chat and file previews', () => {
  it('keeps ordinary prose readable without importing KaTeX from either surface', () => {
    const { container } = render(
      <>
        <MarkdownTextContent isRunning={false} text="Plain chat reply." />
        <MarkdownPreview text="Plain preview document." />
      </>
    )

    expect(container.textContent).toContain('Plain chat reply.')
    expect(container.textContent).toContain('Plain preview document.')
    expect(loaded).not.toHaveBeenCalled()
  })

  it('loads the shared plugin for math and handles a streaming delimiter arriving later', async () => {
    const { container, rerender } = render(<MarkdownTextContent isRunning text="Compute " />)
    rerender(<MarkdownTextContent isRunning text="Compute $" />)
    rerender(<MarkdownTextContent isRunning={false} text="Compute $x^2 + y^2$." />)

    await waitFor(() => expect(container.querySelector('.katex')).not.toBeNull())
    expect(loaded).toHaveBeenCalledOnce()

    rerender(<MarkdownPreview text={'A fraction \\(\\frac{a}{b}\\) here.'} />)
    await waitFor(() => expect(container.querySelector('.katex')).not.toBeNull())
    expect(loaded).toHaveBeenCalledOnce()
  })

  it.each([
    'Block:\n\n$$E = mc^2$$',
    '\\[E = mc^2\\]',
    '$$\\begin{aligned}x &= y\\end{aligned}$$',
    '```math\nx^2\n```'
  ])('renders the supported math form %s', async text => {
    const { container } = render(<MarkdownTextContent isRunning={false} text={text} />)
    await waitFor(() => expect(container.querySelector('.katex')).not.toBeNull())
  })

  it('preserves readable undelimited LaTeX, matching the previous eager pipeline', async () => {
    const text = '\\begin{equation}E = mc^2\\end{equation}'
    const { createMemoizedMathPlugin } = await import('@/lib/katex-memo')

    const eager = render(
      <Streamdown mode="static" plugins={{ math: createMemoizedMathPlugin({ singleDollarTextMath: true }) }}>
        {preprocessMarkdown(text)}
      </Streamdown>
    )

    expect(eager.container.querySelector('.katex')).toBeNull()

    const previous = eager.container.textContent
    eager.unmount()

    const lazy = render(<MarkdownTextContent isRunning={false} text={text} />)

    expect(lazy.container.textContent).toBe(previous)
  })
})
