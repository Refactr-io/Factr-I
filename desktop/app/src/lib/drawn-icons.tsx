import type { SVGProps } from 'react'

// Two glyphs the icon packs here draw differently: Lucide's list-tree (the session rail) and bot-message-square (bots),
// kept as drawn so they match the rest of the Lucide-style marks. Same props as the Tabler icons they sit beside.
type IconProps = Omit<SVGProps<SVGSVGElement>, 'stroke'> & { size?: number | string; stroke?: number | string }

const drawn =
  (paths: string[]) =>
  ({ size = 16, stroke = 1.5, ...props }: IconProps) => (
    <svg
      fill="none"
      height={size}
      stroke="currentColor"
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth={stroke}
      viewBox="0 0 24 24"
      width={size}
      xmlns="http://www.w3.org/2000/svg"
      {...props}
    >
      {paths.map(d => (
        <path d={d} key={d} />
      ))}
    </svg>
  )

export const ListTree = drawn(['M8 5h13', 'M13 12h8', 'M13 19h8', 'M3 10a2 2 0 0 0 2 2h3', 'M3 5v12a2 2 0 0 0 2 2h3'])

export const BotMessageSquare = drawn([
  'M12 6V2H8',
  'M15 11v2',
  'M2 12h2',
  'M20 12h2',
  'M20 16a2 2 0 0 1-2 2H8.828a2 2 0 0 0-1.414.586l-2.202 2.202A.71.71 0 0 1 4 20.286V8a2 2 0 0 1 2-2h12a2 2 0 0 1 2 2z',
  'M9 11v2'
])
