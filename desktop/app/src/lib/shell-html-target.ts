/**
 * A model often builds a web page with one shell command (`cat > index.html <<EOF`, `printf ... > index.html`, `tee`) rather
 * than with a file-writing tool. The page is the same deliverable, so the HTML file such a command writes is a preview
 * target like any other. Plain text matching on purpose: no quoting rules are interpreted, and anything not recognised
 * simply gives no target.
 */

const NAME = String.raw`((?:\\.|[^\s'"|;&<>])+\.html?)`
const REDIRECT = new RegExp(String.raw`(?:^|[\s;&|])\d?>>?\s*(['"]?)${NAME}\1`, 'g')
const TEE = new RegExp(String.raw`\btee\s+(?:-a\s+)?(['"]?)${NAME}\1`, 'g')
const LEADING_CD = /^\s*cd\s+(['"]?)([^\s'"&;|]+)\1\s*(?:&&|;)/

/** The HTML file a shell command writes (`index.html` when it writes several), or '' when it writes none. */
export function htmlFileWrittenByShell(command: string): string {
  const written: string[] = []

  for (const pattern of [REDIRECT, TEE]) {
    pattern.lastIndex = 0

    for (const match of command.matchAll(pattern)) {
      const file = match[2]?.replace(/\\(.)/g, '$1')

      if (file && !/^(?:https?|data|blob):/i.test(file)) {
        written.push(file)
      }
    }
  }

  const file = written.find(f => /(?:^|\/)index\.html?$/i.test(f)) ?? written[0]

  if (!file) {
    return ''
  }

  const dir = LEADING_CD.exec(command)?.[2]

  return dir && dir.startsWith('/') && !file.startsWith('/') && !file.startsWith('~')
    ? `${dir.replace(/\/+$/, '')}/${file.replace(/^\.\//, '')}`
    : file
}
