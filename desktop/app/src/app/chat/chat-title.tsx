import type * as React from 'react'
import { useState } from 'react'

import { Button } from '@/components/ui/button'
import { Codicon } from '@/components/ui/codicon'
import { Tip } from '@/components/ui/tooltip'
import { useI18n } from '@/i18n'
import { cn } from '@/lib/utils'

import { SessionActionsMenu } from './sidebar/session-actions-menu'

interface ChatTitleProps {
  onArchive?: () => void
  onBranch?: () => void
  onDelete?: () => void
  onPin?: () => void
  pinned: boolean
  profile?: string
  sessionId: string
  title: string
}

/** The menu trigger. A component of its own so the dropdown's trigger props and ref reach the button through the tip. */
function MoreTrigger({ label, ...props }: { label: string } & React.ComponentProps<typeof Button>) {
  return (
    <Tip label={label}>
      <Button aria-label={label} type="button" variant="ghost" {...props}>
        <Codicon name="chevron-down" size="0.8125rem" />
      </Button>
    </Tip>
  )
}

/**
 * The chat title as two controls: the title itself renames the chat (tooltip "Rename"), and the chevron beside it
 * opens the session menu (tooltip "More options for <title>").
 */
export function ChatTitle({ onArchive, onBranch, onDelete, onPin, pinned, profile, sessionId, title }: ChatTitleProps) {
  const { t } = useI18n()
  const r = t.sidebar.row
  const [renameOpen, setRenameOpen] = useState(false)
  const controlClass = 'pointer-events-auto h-6 rounded-md text-foreground [-webkit-app-region:no-drag]'

  return (
    <div className="pointer-events-auto flex min-w-0 items-center gap-0.5">
      <Tip label={r.rename.replace(/…$/, '')}>
        <Button
          className={cn(controlClass, 'min-w-0 max-w-full justify-start px-2 text-[0.8125rem] font-medium')}
          onClick={() => setRenameOpen(true)}
          type="button"
          variant="ghost"
        >
          <span className="truncate">{title}</span>
        </Button>
      </Tip>
      <SessionActionsMenu
        align="start"
        onArchive={onArchive}
        onBranch={onBranch}
        onDelete={onDelete}
        onPin={onPin}
        onRenameOpenChange={setRenameOpen}
        pinned={pinned}
        profile={profile}
        renameOpen={renameOpen}
        sessionId={sessionId}
        sideOffset={6}
        surface="header"
        title={title}
      >
        <MoreTrigger
          className={cn(controlClass, 'w-6 shrink-0 px-0 text-(--ui-text-tertiary) hover:text-foreground')}
          label={r.moreOptionsFor(title)}
        />
      </SessionActionsMenu>
    </div>
  )
}
