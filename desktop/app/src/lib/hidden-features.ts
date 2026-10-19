/**
 * Features that ship in the bundle but are hidden from every surface (rail, sidebar, Settings, Capabilities,
 * command palette, composer). Add an id here and the plugin stops registering.
 *
 * Bot Mode was hidden in 0.0.8 because every bot was routed to the single default engine. In 0.0.10 each bot gets its
 * own engine (a container when Docker is available, else its own host process), so it is back.
 */
export const HIDDEN_BUNDLED_PLUGINS: ReadonlySet<string> = new Set()

export const botsHidden = HIDDEN_BUNDLED_PLUGINS.has('factr-bots')
