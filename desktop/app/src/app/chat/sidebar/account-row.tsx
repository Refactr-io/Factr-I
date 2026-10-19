import { useNavigate } from 'react-router'

import { ProfilePicture } from '@/components/ui/profile-picture'
import { Tip, TipKeybindLabel } from '@/components/ui/tooltip'
import { useI18n } from '@/i18n'
import { ChevronUp, RAIL_ICON_PROPS, Settings } from '@/lib/icons'

import { SETTINGS_ROUTE } from '../../routes'

import { ProfileSwitcher } from './profile-dropdown-switcher'

export function AccountRow() {
  const { t } = useI18n()
  const navigate = useNavigate()
  return (
    <div className="flex min-w-0 shrink-0 items-center gap-1 p-2" data-tour="account-row">
      <ProfileSwitcher contentAlign="start" contentSide="top" wrapperClassName="min-w-0 flex-1"
        renderTrigger={({ connectionId, label, name, title, tone }) => (
          <button aria-label={label} className="flex min-h-11 w-full min-w-0 items-center gap-2 rounded-md px-2 text-left transition-colors duration-150 ease-out hover:bg-(--ui-control-hover-background) focus-visible:outline-2 focus-visible:outline-ring [-webkit-app-region:no-drag]" data-tour="profile-switcher" type="button">
            <ProfilePicture connectionId={connectionId} name={name || 'default'} tone={tone} />
            <span className="min-w-0 flex-1">
              <span className="block truncate text-sm font-medium text-foreground">{title}</span>
              <span className="block truncate text-xs text-(--ui-text-secondary)">{t.account.profile(name || 'default')}</span>
            </span>
            <ChevronUp aria-hidden className="size-3.5 shrink-0" stroke={1.5} />
          </button>
        )} />
      <Tip label={<TipKeybindLabel actionId="nav.settings" text={t.sidebar.nav.settings} />}>
        <button aria-label={t.sidebar.nav.settings} className="grid size-9 shrink-0 place-items-center rounded-md text-(--ui-text-secondary) transition-colors duration-150 ease-out hover:bg-(--ui-control-hover-background) hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring [-webkit-app-region:no-drag]" data-tour="nav-settings" onClick={() => navigate(SETTINGS_ROUTE)} type="button">
          <Settings aria-hidden {...RAIL_ICON_PROPS} />
        </button>
      </Tip>
    </div>
  )
}
