import { useStore } from '@nanostores/react'
import { type ReactNode, useEffect, useState } from 'react'

import { useSessionView } from '@/app/chat/session-view'
import { ContextMenu, ContextMenuContent, ContextMenuItem, ContextMenuTrigger } from '@/components/ui/context-menu'
import { findPathRefs, type PathRef, resolveExistingFile } from '@/lib/file-paths'
import { openPreviewTargetInBrowser } from '@/lib/local-preview'
import { notifyError } from '@/store/notifications'
import { openPreview, type PreviewTarget } from '@/store/preview'

/** Blue, underlined link for a path that really exists; plain children otherwise. */
export function FilePathLink({ children, pathRef }: { children: ReactNode; pathRef: PathRef }) {
  const cwd = useStore(useSessionView().$cwd)
  const [target, setTarget] = useState<PreviewTarget | null>(null)

  useEffect(() => {
    let live = true

    void resolveExistingFile(pathRef.path, cwd).then(t => live && setTarget(t))

    return () => {
      live = false
    }
  }, [pathRef.path, cwd])

  if (!target) {
    return <>{children}</>
  }

  const external = () => openPreviewTargetInBrowser(target).catch(e => notifyError(e, 'Could not open file'))

  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>
        <a
          className="ref file-link cursor-pointer wrap-anywhere"
          data-ref="file"
          href="#"
          onClick={e => {
            e.preventDefault()
            void (e.metaKey || e.ctrlKey ? external() : openPreview(target))
          }}
          style={{ textDecorationLine: 'underline', textUnderlineOffset: '0.15em' }}
          title={`${target.path} (Cmd/Ctrl-click: open in default app)`}
        >
          {children}
        </a>
      </ContextMenuTrigger>
      <ContextMenuContent>
        <ContextMenuItem onSelect={() => openPreview(target)}>Open preview</ContextMenuItem>
        <ContextMenuItem onSelect={() => void external()}>Open in default app</ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  )
}

/** Split string children into text + FilePathLinks (each self-verifies existence). */
export function linkifyPaths(children: ReactNode): ReactNode {
  const walk = (node: ReactNode, k: number): ReactNode => {
    if (typeof node !== 'string' || !/[/.]/.test(node)) {
      return node
    }

    const hits = findPathRefs(node)

    if (!hits.length) {
      return node
    }

    const parts: ReactNode[] = []
    let at = 0

    hits.forEach((h, i) => {
      parts.push(node.slice(at, h.start), <FilePathLink key={`${k}-${i}`} pathRef={h.ref}>{h.text}</FilePathLink>)
      at = h.end
    })
    parts.push(node.slice(at))

    return parts
  }

  return Array.isArray(children) ? children.map(walk) : walk(children, 0)
}
