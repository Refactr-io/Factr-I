import { describe, expect, it } from 'vitest'

import { artifactLabel } from './artifact-utils'

describe('artifactLabel', () => {
  it('decodes percent-encoded names from file URLs, https URLs and plain paths', () => {
    expect(artifactLabel('file:///docs/Fundamentals%20of%20Electric%20Circuits.pdf')).toBe(
      'Fundamentals of Electric Circuits.pdf'
    )
    expect(artifactLabel('https://x.test/a/My%20Report%20%281%29.pdf?x=1')).toBe('My Report (1).pdf')
    expect(artifactLabel('/home/me/Fundamentals%20of%20Electric%20Circuits')).toBe('Fundamentals of Electric Circuits')
  })

  it('keeps malformed sequences and ordinary names as they are', () => {
    expect(artifactLabel('/tmp/100%.pdf')).toBe('100%.pdf')
    expect(artifactLabel('file:///tmp/bad%E0%A4%A.txt')).toBe('bad%E0%A4%A.txt')
    expect(artifactLabel('/tmp/plain name.docx')).toBe('plain name.docx')
  })

  it('shows only the file name for Windows drive and UNC paths', () => {
    expect(artifactLabel('C:\\Users\\me\\Fundamentals%20of%20Circuits.pdf')).toBe('Fundamentals of Circuits.pdf')
    expect(artifactLabel('C:\\Users\\me\\plain name.docx')).toBe('plain name.docx')
    expect(artifactLabel('\\\\server\\share\\a%20b.txt')).toBe('a b.txt')
    expect(artifactLabel('file:///C:/Users/me/a%20b.pdf')).toBe('a b.pdf')
  })
})
