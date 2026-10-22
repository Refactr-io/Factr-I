/** The line the engine appends to a shell result whose command ran through the `rtk` output reducer. */
export const OUTPUT_REDUCED_MARKER = '[output reduced by rtk]'

/** Whether a tool result (a string, or an object carrying output text) ends with the reducer's marker. */
export function isOutputReduced(result: unknown): boolean {
  const text =
    typeof result === 'string'
      ? result
      : result && typeof result === 'object'
        ? String(
            (result as Record<string, unknown>).output ??
              (result as Record<string, unknown>).result_text ??
              (result as Record<string, unknown>).stdout ??
              ''
          )
        : ''

  return text.trimEnd().endsWith(OUTPUT_REDUCED_MARKER)
}
