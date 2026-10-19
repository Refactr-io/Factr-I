import { Loader } from './loader'

/** Compatibility for existing sidebar and plugin callers. */
export function LatticeLoader({
  pattern: _pattern,
  ...props
}: Omit<React.ComponentProps<typeof Loader>, 'variant'> & { pattern?: 'orbit' | 'ripple' | 'snake' | 'spiral' }) {
  return <Loader {...props} />
}
