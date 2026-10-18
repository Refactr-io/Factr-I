import { requestForBot } from './routing'
import type { BotRequestOptions } from './routing'
import type { CanonicalSession, RosterRow } from './types'

/** A bot's own engine keeps its sessions in its own store; the roster's `canonical_session` is read from the
 *  primary's state.db, which never sees them, so it is null for every engine-backed bot. The engine serves the
 *  registry itself (`bot.chat.get`): `undefined` means this engine cannot answer (an older engine, a Python
 *  gateway, unreachable), `null` that it answered and has no Bot Chat yet. */
export async function engineBotChat(
  bot: Partial<RosterRow>,
  options?: BotRequestOptions
): Promise<CanonicalSession | null | undefined> {
  try {
    const res = await requestForBot<{ canonical_session?: CanonicalSession | null }>(bot, 'bot.chat.get', {}, options)

    return res && typeof res === 'object' && 'canonical_session' in res ? (res.canonical_session ?? null) : undefined
  } catch {
    return undefined
  }
}

/** Register a Bot Chat the desktop just created on this engine. The registered chat comes back: the new one, or
 *  the one that was already there (the first wins). `undefined` when the engine cannot answer. */
export async function engineClaimBotChat(bot: Partial<RosterRow>, sessionId: string): Promise<CanonicalSession | undefined> {
  try {
    const res = await requestForBot<{ canonical_session?: CanonicalSession | null }>(bot, 'bot.chat.claim', { session_id: sessionId })

    return res?.canonical_session ?? undefined
  } catch {
    return undefined
  }
}

/** What the bots' engines last said about their Bot Chat, kept for the roster. Filled from the primary's batch
 *  answer and from the open path; the roster itself never asks a bot's engine, because that would start it. */
const chats = new Map<string, { at: number; chat: CanonicalSession | null }>()

/** `connection::profile`: the row and the open path name the same bot the same way. */
export const engineChatKey = (bot: Partial<RosterRow>) => `${bot.connectionId ?? 'local'}::${bot.name ?? ''}`

export const cachedEngineBotChat = (bot: Partial<RosterRow>) => chats.get(engineChatKey(bot))

/** Remember an answer. A `null` (the asker did not know the chat) never erases a chat that was learned. */
export function rememberEngineBotChat(bot: Partial<RosterRow>, chat: CanonicalSession | null, at = Date.now()): boolean {
  const key = engineChatKey(bot)
  const before = chats.get(key)?.chat ?? null

  chats.set(key, { at, chat: chat ?? before })

  return (chat?.id ?? null) !== (before?.id ?? null)
}

/** Test seam. */
export function resetEngineBotChats(): void {
  chats.clear()
}
