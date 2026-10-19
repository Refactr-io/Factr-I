import type { BrailleSpinnerName as SpinnerName } from 'unicode-animations'

import { Loader } from './loader'

export type { SpinnerName }

/** Keep the SDK prop contract while using the app's shared pixel loader. */
export function GlyphSpinner({
  spinner: _spinner,
  ...props
}: Omit<React.ComponentProps<typeof Loader>, 'variant'> & { spinner?: SpinnerName }) {
  return <Loader {...props} ariaLabel={props.ariaLabel ?? 'Loading'} />
}
