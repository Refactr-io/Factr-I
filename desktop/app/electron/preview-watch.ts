import path from 'node:path'

/**
 * What a preview file watch reacts to. A tab on an HTML page also shows the stylesheets, scripts and images the page
 * loads from its folder, and an agent usually writes `index.html` first and the assets after it: the page has to reload
 * when those land, not only when the HTML file itself changes. Any other file type, or any other file next to a
 * non-HTML preview, is ignored.
 */
const HTML = /\.html?$/i
const PAGE_ASSET = /\.(?:css|js|mjs|json|svg|png|jpe?g|gif|webp|avif|ico|woff2?|ttf|otf|html?)$/i

export function previewWatchWantsRecursive(targetName: string): boolean {
  return HTML.test(targetName)
}

export function shouldReloadPreviewFor(targetName: string, changedPath: string): boolean {
  if (!changedPath) {
    return true
  }

  const changed = path.basename(changedPath)

  return changed === targetName || (HTML.test(targetName) && PAGE_ASSET.test(changed))
}
