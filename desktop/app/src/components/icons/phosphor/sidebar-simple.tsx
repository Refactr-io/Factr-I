// ph:sidebar-simple
import type { SVGProps } from 'react'

export function SidebarSimple(props: SVGProps<SVGSVGElement>) {
  return (
    <svg height="1em" viewBox="0 0 256 256" width="1em" xmlns="http://www.w3.org/2000/svg" {...props}>
      <path
        d="M216 40H40a16 16 0 0 0-16 16v144a16 16 0 0 0 16 16h176a16 16 0 0 0 16-16V56a16 16 0 0 0-16-16M40 56h40v144H40Zm176 144H96V56h120z"
        fill="currentColor"
      />
    </svg>
  )
}
