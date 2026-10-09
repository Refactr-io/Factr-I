import { revealTreePane } from '@/components/pane-shell/tree/store'
import { useI18n } from '@/i18n'
import { FileText, GitBranch, Terminal } from '@/lib/icons'

type Strings = ReturnType<typeof useI18n>['t']

const TOOLS = [
  { icon: FileText, id: 'files', label: (t: Strings) => t.sidebar.files },
  { icon: Terminal, id: 'terminal', label: (t: Strings) => t.sidebar.terminal },
  { icon: GitBranch, id: 'review', label: (t: Strings) => t.sidebar.review }
] as const

/** A blank Browser tab's page: the right sidebar's tools, one click each. */
export function BrowserTools() {
  const { t } = useI18n()

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto px-6 pt-8">
      <div className="text-[0.8125rem] font-medium text-foreground/85">Tools</div>
      <div className="grid grid-cols-1 gap-1.5">
        {TOOLS.map(({ icon: Icon, id, label }) => (
          <button
            className="flex h-9 items-center gap-2.5 rounded-lg bg-(--ui-bg-tertiary) px-3 text-left text-[0.8125rem] text-foreground/85 transition-colors hover:bg-(--chrome-action-hover) hover:text-foreground"
            key={id}
            onClick={() => revealTreePane(id)}
            type="button"
          >
            <Icon className="size-4 shrink-0 text-(--ui-text-tertiary)" />
            {label(t)}
          </button>
        ))}
      </div>
      <p className="m-0 pt-2 text-xs leading-relaxed text-(--ui-text-tertiary)">
        Type an address above to browse, or ask Factr-I to open a page.
      </p>
    </div>
  )
}
