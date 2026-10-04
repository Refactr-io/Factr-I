import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

// Collect the component graph before the behavioral test deadline starts.
import { GatewaySettings } from './gateway-settings'

const { registry, activeId, selectConnection } = vi.hoisted(() => ({
  registry: { value: null as any },
  activeId: { value: 'saved-b' },
  selectConnection: vi.fn().mockResolvedValue(undefined)
}))

vi.mock('@nanostores/react', () => ({ useStore: (store: any) => store.value }))
vi.mock('@/store/connections', () => ({
  $connectionsRegistry: registry,
  $activeConnectionId: activeId,
  refreshConnectionsRegistry: vi.fn().mockResolvedValue(null),
  selectConnection,
  setConnectionsRegistry: vi.fn()
}))
vi.mock('./connections-registry', async importOriginal => ({
  ...(await importOriginal<any>()),
  ConnectionsRegistrySection: () => null
}))
const getConnectionConfig = vi.fn()
const saveConnectionConfig = vi.fn()

// This test owns the machine-level GatewaySettings contract. The managed SSH
// update section mounted below the registry has its own focused coverage
// (store/managed-updates.test.ts); keep its store subscriptions out of this
// single-purpose test.
vi.mock('./managed-updates-section', () => ({ ManagedUpdatesSection: () => null }))

const localConnection = {
  cloudOrg: '',
  envOverride: false,
  mode: 'local',
  remoteAuthMode: 'token',
  remoteOauthConnected: false,
  remoteTokenPreview: null,
  remoteTokenSet: false,
  remoteUrl: ''
}

beforeEach(() => {
  getConnectionConfig.mockResolvedValue(localConnection)
  saveConnectionConfig.mockResolvedValue(localConnection)
  Object.defineProperty(window, 'factrDesktop', {
    configurable: true,
    value: { getConnectionConfig, saveConnectionConfig }
  })
})

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe('GatewaySettings', () => {
  // #114856: an env-pinned remote (FACTR_DESKTOP_REMOTE_URL) whose session
  // lapsed could not be re-authenticated from Settings → Gateway at all — the
  // whole remote block (URL + probe + Authentication) was hidden behind
  // `!state.envOverride`, so the recovery card's "Gateway settings" escape led
  // to a read-only banner with no sign-in, leaving "Use local gateway" as the
  // only way back in. The env override pins the URL, not the browser session
  // (docs: "you still sign in from the Gateway settings panel"), so the
  // Authentication row must stay reachable — its controls are not env-owned.
  describe('env-override remote', () => {
    const envUrl = 'http://100.116.104.53:9191'

    const envRemote = {
      ...localConnection,
      envOverride: true,
      mode: 'remote',
      remoteAuthMode: 'token',
      remoteUrl: envUrl
    }

    const oauthProbe = {
      authMode: 'oauth',
      providers: [{ displayName: 'Example IdP', name: 'example', supportsPassword: false }],
      reachable: true
    }

    it('reaches sign-in for a lapsed session instead of a dead env banner', async () => {
      const oauthLoginConnectionConfig = vi.fn().mockResolvedValue({ connected: true })
      const probeConnectionConfig = vi.fn().mockResolvedValue(oauthProbe)

      getConnectionConfig.mockResolvedValue({ ...envRemote, remoteOauthConnected: false })
      // Sign-in persists the URL + oauth mode before opening the login window;
      // the saved echo must stay remote or the signing sequence resets.
      saveConnectionConfig.mockResolvedValue({ ...envRemote, remoteAuthMode: 'oauth' })
      Object.assign(window.factrDesktop, { oauthLoginConnectionConfig, probeConnectionConfig })

      render(<GatewaySettings embedded />)

      // The env override still owns the URL: the editor stays read-only.
      expect(((await screen.findByDisplayValue(envUrl)) as HTMLInputElement).disabled).toBe(true)

      fireEvent.click(await screen.findByRole('button', { name: 'Sign in with Example IdP' }))

      await waitFor(() => expect(oauthLoginConnectionConfig).toHaveBeenCalledWith(envUrl))
    })

    it('leaves a saved (non-env) remote session editable and unchanged', async () => {
      const oauthLoginConnectionConfig = vi.fn()

      getConnectionConfig.mockResolvedValue({ ...envRemote, envOverride: false, remoteOauthConnected: false })
      Object.assign(window.factrDesktop, {
        oauthLoginConnectionConfig,
        probeConnectionConfig: vi.fn().mockResolvedValue(oauthProbe)
      })

      render(<GatewaySettings embedded />)

      expect(((await screen.findByDisplayValue(envUrl)) as HTMLInputElement).disabled).toBe(false)
      expect(await screen.findByRole('button', { name: 'Sign in with Example IdP' })).toBeTruthy()
      expect(oauthLoginConnectionConfig).not.toHaveBeenCalled()
    })
  })

  it('loads the machine-level connection config (no profile scoping)', async () => {
    render(<GatewaySettings />)
    expect(await screen.findByText('Local gateway')).toBeTruthy()

    // The page manages the machine's gateway connections; it must load the
    // global config, never a per-profile override.
    await waitFor(() => expect(getConnectionConfig).toHaveBeenCalledWith(null))
    expect(getConnectionConfig).not.toHaveBeenCalledWith(expect.any(String))

    // The legacy per-profile scope switcher must not render.
    expect(screen.queryByText('Applies to')).toBeNull()
    expect(screen.queryByText('All profiles')).toBeNull()
    expect(screen.queryByText('Use default gateway')).toBeNull()
  })
})
