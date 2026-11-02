import { fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { SyntaxHighlighter } from '@/components/chat/shiki-highlighter'

import { isShellLanguage } from './run-command-button'

const Pre = (props: React.ComponentProps<'pre'>) => <pre {...props} />

const block = (language: string, code = 'ls -la') =>
  render(<SyntaxHighlighter code={code} components={{ Pre } as never} defer language={language} node={undefined as never} />)

const run = vi.fn(async (_o: { command: string; cwd?: string }) => null as null | string)

afterEach(() => {
  run.mockClear()
  delete (window as any).factrDesktop
})

describe('isShellLanguage', () => {
  it('matches shell fences only', () => {
    for (const l of ['bash', 'sh', 'zsh', 'shell', 'Bash']) {
      expect(isShellLanguage(l)).toBe(true)
    }

    for (const l of ['ts', 'python', 'json', '', undefined]) {
      expect(isShellLanguage(l)).toBe(false)
    }
  })
})

describe('Run button on code blocks', () => {
  it('shows on shell blocks and not on others', () => {
    const { unmount } = block('bash')
    expect(screen.queryByRole('button', { name: 'Run in terminal' })).not.toBeNull()
    unmount()
    block('python')
    expect(screen.queryByRole('button', { name: 'Run in terminal' })).toBeNull()
  })

  it('never runs on render; runs once via IPC on click', async () => {
    ;(window as any).factrDesktop = { terminal: { run, onData: vi.fn(), onExit: vi.fn() } }
    block('sh', 'echo hi')
    expect(run).not.toHaveBeenCalled()
    fireEvent.click(screen.getByRole('button', { name: 'Run in terminal' }))
    await Promise.resolve()
    expect(run).toHaveBeenCalledTimes(1)
    expect(run.mock.calls[0][0].command).toBe('echo hi')
  })
})
