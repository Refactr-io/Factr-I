import { useI18n } from '@/i18n'

/** A blank Browser tab's page. The tools live in the pane cluster now, so the page only says what to do. */
export function BrowserTools() {
  const { t } = useI18n()

  return (
    <div className="grid min-h-0 flex-1 place-items-center px-6 text-center text-xs leading-relaxed text-(--ui-text-tertiary)">
      {t.preview.web.blankPageBody}
    </div>
  )
}
