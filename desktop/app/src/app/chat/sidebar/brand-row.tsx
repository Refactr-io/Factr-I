import { Button } from '@/components/ui/button'
import { Codicon } from '@/components/ui/codicon'
import { Tip } from '@/components/ui/tooltip'
import { useI18n } from '@/i18n'
import { PRODUCT_NAME } from '@/lib/brand'
import { requestSessionSearchFocus } from '@/store/layout'

/**
 * The panel's top row (registered as `zoneChrome.lead`): the product name as
 * plain text and a search button that opens the session search field. The
 * profile switcher lives on the rail's avatar; there is no bell because the app
 * has no notifications surface.
 */
export function SidebarBrandRow() {
  const { t } = useI18n()

  return (
    <div className="flex w-full min-w-0 items-center gap-1 pr-2 pl-2.5">
      <span className="min-w-0 flex-1 truncate px-1.5 text-base font-semibold tracking-tight text-foreground">
        {PRODUCT_NAME}
      </span>
      <Tip label={t.sidebar.searchAria}>
        <Button
          aria-label={t.sidebar.searchAria}
          className="size-7 shrink-0 text-(--ui-text-tertiary) hover:bg-(--ui-control-hover-background) hover:text-foreground [-webkit-app-region:no-drag]"
          onClick={requestSessionSearchFocus}
          size="icon-xs"
          variant="ghost"
        >
          <Codicon name="search" size="0.875rem" />
        </Button>
      </Tip>
    </div>
  )
}
