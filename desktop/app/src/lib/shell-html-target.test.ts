import { describe, expect, it } from 'vitest'

import { htmlFileInPatch, htmlFileWrittenByShell } from './shell-html-target'

describe('the web page a shell command writes', () => {
  it('finds a heredoc and a printf redirect', () => {
    expect(htmlFileWrittenByShell("cat > /tmp/site/index.html <<'EOF'\n<h1>x</h1>\nEOF")).toBe('/tmp/site/index.html')
    expect(htmlFileWrittenByShell("mkdir -p /tmp/s && printf '%s\\n' '<h1>x</h1>' > /tmp/s/index.html")).toBe('/tmp/s/index.html')
  })

  it('finds tee and an append, with quotes', () => {
    expect(htmlFileWrittenByShell('echo hi | tee "/tmp/a b/page.html"')).toBe('')
    expect(htmlFileWrittenByShell("echo hi | tee -a '/tmp/page.html'")).toBe('/tmp/page.html')
    expect(htmlFileWrittenByShell('echo hi >> /tmp/page.htm')).toBe('/tmp/page.htm')
  })

  it('prefers index.html when several pages are written', () => {
    expect(htmlFileWrittenByShell('echo a > /t/about.html && echo b > /t/index.html')).toBe('/t/index.html')
  })

  it('joins a relative file to the folder the command changes into', () => {
    expect(htmlFileWrittenByShell('cd /tmp/site && cat > index.html <<EOF\nx\nEOF')).toBe('/tmp/site/index.html')
    expect(htmlFileWrittenByShell('cat > index.html <<EOF\nx\nEOF')).toBe('index.html')
  })

  it('ignores commands that only mention or read a page', () => {
    expect(htmlFileWrittenByShell('cat /tmp/site/index.html')).toBe('')
    expect(htmlFileWrittenByShell('open /tmp/site/index.html')).toBe('')
    expect(htmlFileWrittenByShell('echo hello > /tmp/notes.txt')).toBe('')
    expect(htmlFileWrittenByShell('curl -o page.html https://example.com/index.html')).toBe('')
  })
})

describe('the web page a patch adds', () => {
  it('reads the path of an apply_patch Add File or Update File', () => {
    expect(htmlFileInPatch('*** Begin Patch\n*** Add File: /tmp/site/index.html\n+<h1>x</h1>\n*** Add File: /tmp/site/style.css\n+b{}\n*** End Patch')).toBe('/tmp/site/index.html')
    expect(htmlFileInPatch('*** Begin Patch\n*** Update File: site/page.htm\n@@\n-a\n+b\n*** End Patch')).toBe('site/page.htm')
  })

  it('reads a unified diff and ignores non-HTML files', () => {
    expect(htmlFileInPatch('--- a/x.html\n+++ b/x.html\n@@\n-a\n+b')).toBe('x.html')
    expect(htmlFileInPatch('*** Begin Patch\n*** Add File: /tmp/app.js\n+1\n*** End Patch')).toBe('')
  })
})
