import { atom } from 'nanostores'

/**
 * Launch-flag gate for every local-models surface in the GUI.
 *
 * Local models ship on main behind `--local` (either `factr desktop --local`
 * or the flag on Factr.exe itself). The flag is strict: without it the GUI
 * shows no local-models surface at all, even on a machine where local models
 * are configured and running — the backend routes stay live, only the
 * desktop's presentation is gated. Read once from the preload bridge at
 * module load; a launch flag can't change mid-session, so nothing rewrites
 * it outside tests.
 */
export const $localModelsEnabled = atom<boolean>(
  typeof window !== 'undefined' && window.factrDesktop?.localModelsEnabled === true
)
