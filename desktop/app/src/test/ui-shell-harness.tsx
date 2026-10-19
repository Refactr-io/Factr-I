import '@/styles.css'

import { useStore } from '@nanostores/react'
import { atom } from 'nanostores'
// Isolated shell fixture: real layout/chrome, synthetic panes, no gateway or credentials.
import { createRoot } from 'react-dom/client'
import { MemoryRouter } from 'react-router'

import { DEFAULT_TREE, registerLayoutPresets } from '@/app/contrib/layout-presets'
import { TitlebarControls } from '@/app/shell/titlebar-controls'
import { findGroupOfPane } from '@/components/pane-shell/tree/model'
import { applyLayoutPreset } from '@/components/pane-shell/tree/presets'
import { LayoutTreeRoot } from '@/components/pane-shell/tree/renderer'
import {
  $layoutTree,
  bindPaneVisibility,
  bindToolPaneCollapse,
  declareDefaultTree,
  moveTreePane,
  watchContributedPanes
} from '@/components/pane-shell/tree/store'
import { registry } from '@/contrib/registry'
import { I18nProvider } from '@/i18n'
import { ThemeProvider } from '@/themes/context'

for (const [id, title, placement] of [
  ['sessions', 'Sessions', 'left'],
  ['workspace', 'Shell layout fixture', 'main'],
  ['terminal', 'Terminal', 'bottom'],
  ['files', 'Files', 'right'],
  ['review', 'Changes', 'right']
] as const) {
  registry.register({
    id,
    area: 'panes',
    title,
    data: {
      placement,
      width: placement === 'main' ? undefined : '260px',
      minWidth: placement === 'main' ? '22vw' : '180px',
      uncloseable: id === 'workspace',
      lifecycleKeepAlive: id === 'terminal'
    },
    render: () => (
      <div className="h-full overflow-auto p-4" data-session-anchor={id === 'workspace' ? 'workspace' : undefined}>
        <h2 className="mb-4 font-medium">{title}</h2>
        {id === 'workspace' ? (
          <>
            <p>This fixture uses the production pane tree and titlebar with synthetic session content.</p>
            <p className="mt-4">Open tools, resize the window, move panes and restore focus.</p>
            <textarea
              aria-label="Fixture composer"
              className="mt-8 w-full rounded-md border p-3"
              defaultValue="A readable conversation stays in its own column."
            />
          </>
        ) : (
          <p>
            {id === 'terminal'
              ? 'factr-i $ ready'
              : id === 'sessions'
                ? 'Shell QA session'
                : 'No files changed in this synthetic session.'}
          </p>
        )}
      </div>
    )
  })
}

registerLayoutPresets()
declareDefaultTree(DEFAULT_TREE)
watchContributedPanes()

for (const id of ['files', 'review']) {
  const open = atom(true)
  bindPaneVisibility(
    id,
    open,
    () => open.set(false),
    () => open.set(true)
  )
}

const terminal = atom(true)
bindToolPaneCollapse(
  'terminal',
  terminal,
  () => terminal.set(false),
  () => terminal.set(true)
)

function Fixture() {
  const tree = useStore($layoutTree)

  return (
    <div
      className="flex h-screen flex-col"
      style={
        {
          '--titlebar-controls-left': '14px',
          '--titlebar-controls-top': '8px',
          '--titlebar-tools-right': '14px',
          '--titlebar-control-size': '24px',
          '--titlebar-height': '40px'
        } as React.CSSProperties
      }
    >
      <TitlebarControls onOpenSettings={() => alert('Settings fixture')} />
      <div className="flex min-h-0 flex-1" data-shell-block="">
        <LayoutTreeRoot titlebar />
      </div>
      <div className="flex gap-4 p-2 text-xs">
        <button
          onClick={() => {
            const preset = registry.getArea('layouts').find(p => p.id === 'basic')!
            applyLayoutPreset('basic', preset.data as NonNullable<typeof tree>)
          }}
        >
          Apply Basic fixture
        </button>
        <button
          onClick={() => {
            const main = tree && findGroupOfPane(tree, 'workspace')

            if (main) {moveTreePane('terminal', { groupId: main.id, pos: 'left' })}
          }}
        >
          Move terminal fixture
        </button>
      </div>
    </div>
  )
}

createRoot(document.getElementById('root')!).render(
  <MemoryRouter>
    <I18nProvider configClient={null} initialLocale="en">
      <ThemeProvider>
        <Fixture />
      </ThemeProvider>
    </I18nProvider>
  </MemoryRouter>
)
