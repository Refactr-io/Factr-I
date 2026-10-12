/**
 * Features that ship in the bundle but are hidden from every surface (rail, sidebar, Settings, Capabilities,
 * command palette, composer). Bot Mode is hidden in 0.0.8: in the installed app every bot was routed to the single
 * default engine, so bot chats never woke. The code stays so the feature can come back when it is fixed:
 * remove the id here and the plugin registers again.
 */
export const HIDDEN_BUNDLED_PLUGINS: ReadonlySet<string> = new Set(['factr-bots'])

export const botsHidden = HIDDEN_BUNDLED_PLUGINS.has('factr-bots')
