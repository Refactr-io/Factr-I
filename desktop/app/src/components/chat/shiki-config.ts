// Shiki theme/color constants shared by the chat code-block renderer
// (shiki-highlighter.tsx) and the lazy shiki chunk (shiki-block.tsx). Kept in
// their own dependency-free module so the lazy chunk can import them without
// pulling the main chat module (or react-shiki) into the shiki bundle.

// `github-dark-dimmed` is GitHub's lower-contrast dark palette — the vivid
// `github-dark-default` tokens read harsh at our small code size. Shared by the
// inline diff renderer too (see diff-lines.tsx) so code + diffs match.
export const SHIKI_THEME = { dark: 'github-dark-dimmed', light: 'github-light-default' } as const

/**
 * The two GitHub themes carry cool blue-grey neutrals (text, punctuation, comments). Remap those to the app's warm
 * neutrals so code blocks sit in the same palette as the rest of the UI; the hued token colours (keywords, strings,
 * numbers) are left alone. Light-mode comments also move to a darker grey (about 5.7:1 on the code card; the stock
 * `#6e7781` is 4.2:1 and a single `#` turns a whole shell line into one comment span). Keyed per theme name.
 */
export const SHIKI_COLOR_REPLACEMENTS: Record<string, Record<string, string>> = {
  'github-dark-dimmed': { '#adbac7': '#fffdfc', '#cdd9e5': '#fffdfc', '#768390': '#818581', '#636e7b': '#818581' },
  'github-light-default': { '#1f2328': '#151313', '#24292f': '#151313', '#6e7781': '#575554', '#57606a': '#575554' }
}

/**
 * Cache-key scope for the content-addressed highlight cache. Bumping this
 * invalidates every cached highlight at once — bump it whenever the rendering
 * options (themes, color replacements) change, because keys are NOT allowed to
 * silently produce a different DOM than the one they were computed with.
 */
export const SHIKI_HIGHLIGHT_SCOPE = `factr-shiki-v1:${JSON.stringify({
  dark: SHIKI_THEME.dark,
  light: SHIKI_THEME.light,
  colorReplacements: SHIKI_COLOR_REPLACEMENTS
})}`
