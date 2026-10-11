/** The gateway line under "Messaging gateway running". The status endpoint can omit the version or the
 *  session count (older/partial replies); never print "undefined"/"null" for a missing field. */
export function systemStatusLine(
  format: (version: string, count: number) => string,
  status: { active_sessions?: number | null; version?: string | null }
): string {
  const count = Number.isFinite(status.active_sessions) ? Number(status.active_sessions) : 0

  return format(status.version?.trim() || '', count).replace(/^Factr-I\s{2,}/, 'Factr-I ')
}
