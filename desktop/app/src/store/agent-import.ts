import { atom } from 'nanostores'

/** The "Import from other agents" dialog is mounted once at the app root; every menu just opens it. */
export const $agentImportOpen = atom(false)

export const openAgentImport = () => $agentImportOpen.set(true)

export const closeAgentImport = () => $agentImportOpen.set(false)
