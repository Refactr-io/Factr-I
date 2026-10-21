import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { $pluginRecords } from '@/contrib/plugins-store'
import { $agentPlugins, $agentPluginsStatus } from '@/store/agent-plugins'
import { $confirmRequest, settleConfirm } from '@/store/confirm'
import { $pluginInstallRequest, closePluginInstallRequest } from '@/store/plugin-install-request'
import { $connection } from '@/store/session'

import { PluginsTab } from './plugins-tab'

let catalogRows: Array<Record<string, unknown>> = []

vi.mock('@/lib/plugin-catalog', async importOriginal => ({
  ...(await importOriginal<Record<string, unknown>>()),
  fetchPluginCatalogRows: vi.fn(async () => catalogRows)
}))

function catalogRow(name: string, extra: Record<string, unknown> = {}) {
  return {
    description: '',
    installed: false,
    name,
    repo: `https://github.com/example/${name}`,
    tier: 'community',
    title: name,
    updateAvailable: false,
    ...extra
  }
}

const clickCatalogInstall = async (name: string) => {
  const row = await screen.findByTestId(`catalog-row-${name}`)

  fireEvent.click(within(row).getByRole('button'))
}

const requestGateway = vi.fn(async () => ({ plugins: [] }))

const connectionFixture = {
  baseUrl: 'http://localhost',
  isFullscreen: false,
  logs: [],
  nativeOverlayWidth: 0,
  token: '',
  windowButtonPosition: null,
  wsUrl: ''
}

vi.mock('@/app/gateway/hooks/use-gateway-request', () => ({
  useGatewayRequest: () => ({ requestGateway })
}))

const uninstallDiskPlugin = vi.fn(async (_id: string) => ({ ok: true }))

vi.mock('@/contrib/runtime-loader', async importOriginal => ({
  ...(await importOriginal<Record<string, unknown>>()),
  uninstallDiskPlugin: (id: string) => uninstallDiskPlugin(id)
}))

describe('PluginsTab', () => {
  beforeEach(() => {
    $pluginRecords.set({})
    $agentPlugins.set([])
    $agentPluginsStatus.set('ready')
    closePluginInstallRequest()
    requestGateway.mockClear()
    catalogRows = []
  })

  afterEach(() => {
    cleanup()
    $connection.set(null)
  })

  it('renders declared server pills and the unavailable sentence under the description', () => {
    $agentPlugins.set([
      {
        description: 'A test plugin',
        key: 'demo-plugin',
        name: 'demo-plugin',
        servers: [
          { name: 'ready-server', sentence: '', state: 'connected' },
          {
            name: 'setup-server',
            sentence: 'Example App is not installed. Install Example App, then try again.',
            state: 'missing_app'
          }
        ],
        source: 'git',
        status: 'enabled',
        version: '1.0.0'
      }
    ])

    render(<PluginsTab profile="workbot" />)

    expect(screen.getByTestId('server-pill-ready-server')).toBeTruthy()
    expect(screen.getByTestId('server-pill-setup-server')).toBeTruthy()
    expect(screen.getByText('Example App is not installed. Install Example App, then try again.')).toBeTruthy()
    expect(screen.getByRole('switch', { name: 'Agent: demo-plugin' }).getAttribute('aria-checked')).toBe('true')
  })

  it('hides bundled plugins (managed from their own surfaces)', () => {
    $agentPlugins.set([
      {
        description: '',
        key: 'image_gen/fal',
        name: 'fal',
        source: 'bundled',
        status: 'enabled',
        version: ''
      }
    ])

    render(<PluginsTab profile={null} />)

    expect(screen.queryByText('fal')).toBeNull()
    expect(screen.getByText(/No plugins yet/)).toBeTruthy()
  })

  // A desktop half can only be copied out of a backend that runs on THIS
  // machine; against a remote one the reconcile is a structural no-op, so the
  // row must say so instead of pending forever (#114079).
  it('marks a remote-backend desktop half unavailable instead of forever copying', () => {
    $connection.set({ ...connectionFixture, mode: 'remote' })
    $agentPlugins.set([
      {
        description: '',
        has_desktop_half: true,
        key: 'example-prices',
        name: 'example-prices',
        source: 'catalog',
        status: 'enabled',
        version: '1'
      }
    ])

    render(<PluginsTab profile={null} />)

    const detail = within(screen.getByRole('row', { name: /^example-prices/ }))
    expect(detail.getByText('unavailable (remote backend)')).toBeTruthy()
    expect(detail.queryByText('copying…')).toBeNull()
  })

  it('keeps the pending desktop-half state on a local backend', () => {
    $agentPlugins.set([
      {
        description: '',
        has_desktop_half: true,
        key: 'example-prices',
        name: 'example-prices',
        source: 'catalog',
        status: 'enabled',
        version: '1'
      }
    ])

    render(<PluginsTab profile={null} />)

    const detail = within(screen.getByRole('row', { name: /^example-prices/ }))
    expect(detail.getByText('copying…')).toBeTruthy()
    expect(detail.queryByText('unavailable (remote backend)')).toBeNull()
  })

  it('renders a unified package as ONE row with a Desktop switch and an Agent switch', () => {
    $pluginRecords.set({
      media: { id: 'media', name: 'Media Studio', kind: 'disk', status: 'loaded', packageName: 'factr-media-studio' }
    })
    $agentPlugins.set([
      {
        description: '',
        key: 'factr-media-studio',
        name: 'factr-media-studio',
        source: 'git',
        status: 'disabled',
        version: '1'
      }
    ])

    render(<PluginsTab profile="workbot" scopeLabel="workbot" />)

    expect(screen.getAllByTestId(/^plugin-row-/)).toHaveLength(1)
    expect(screen.getByText('Agent + Desktop')).toBeTruthy()
    expect(screen.getByRole('switch', { name: 'Desktop: Media Studio' }).getAttribute('aria-checked')).toBe('true')
    expect(screen.getByRole('switch', { name: 'Agent: Media Studio' }).getAttribute('aria-checked')).toBe('false')
    expect(screen.getAllByText('Agent in workbot').length).toBeGreaterThan(0)
  })

  it('offers "Install here" for a desktop half whose agent half is not in the selected profile', async () => {
    $pluginRecords.set({
      media: {
        id: 'media',
        name: 'Media Studio',
        kind: 'disk',
        status: 'loaded',
        packageName: 'factr-media-studio',
        packageOrigin: { repo: 'https://github.com/example/factr-media-studio.git', sha: 'abc' }
      }
    })

    render(<PluginsTab profile="workbot" scopeLabel="workbot" />)

    expect(screen.queryByRole('switch', { name: /^Agent:/ })).toBeNull()
    screen.getByRole('button', { name: 'Install here' }).click()
    // Pre-filled from the package marker: repo + pinned sha, agent half only.
    await waitFor(() => {
      expect($pluginInstallRequest.get()).toMatchObject({
        legacyHint: 'agent',
        profile: 'workbot',
        repo: 'https://github.com/example/factr-media-studio.git',
        sha: 'abc'
      })
    })
  })

  it('disables "Install here" when the package has no known origin (hand-copied folder)', () => {
    $pluginRecords.set({
      media: { id: 'media', name: 'Media Studio', kind: 'disk', status: 'loaded', packageName: 'factr-media-studio' }
    })

    render(<PluginsTab profile="workbot" scopeLabel="workbot" />)

    expect((screen.getByRole('button', { name: 'Install here' }) as HTMLButtonElement).disabled).toBe(true)
  })

  it('loads the plugin list scoped to the selected profile', () => {
    render(<PluginsTab profile="workbot" />)

    expect(requestGateway).toHaveBeenCalledWith(
      'plugins.manage',
      expect.objectContaining({ action: 'list', profile: 'workbot' })
    )
  })

  it('opens the dual-target install modal from a catalog row', async () => {
    catalogRows = [catalogRow('weather-plugin', { sha: 'a'.repeat(40) })]
    render(<PluginsTab profile="workbot" />)
    await clickCatalogInstall('weather-plugin')

    await waitFor(() => {
      const request = $pluginInstallRequest.get()

      expect(request).not.toBeNull()
      expect(request?.catalogName).toBe('weather-plugin')
      expect(request?.repo).toBe('https://github.com/example/weather-plugin')
      expect(request?.profile).toBe('workbot')
      expect(request?.sha).toBe('a'.repeat(40))
    })
  })

  it('shows an honest empty state when the backend ships no catalog entries', async () => {
    const { container } = render(<PluginsTab profile={null} />)

    expect(await screen.findByText(/No curated plugins are listed yet/)).toBeTruthy()
    expect(container.querySelector('iframe')).toBeNull()
  })

  it('toggles by canonical key through plugins.manage', async () => {
    $agentPlugins.set([
      {
        description: '',
        key: 'image_gen/legacy',
        name: 'Legacy plugin',
        source: 'user',
        status: 'disabled',
        version: '0.20.0'
      }
    ])
    requestGateway.mockResolvedValueOnce({
      ok: true,
      plugin: { key: 'image_gen/legacy', name: 'Legacy plugin', status: 'enabled' }
    } as never)

    render(<PluginsTab profile={null} />)

    screen.getByRole('switch', { name: 'Agent: Legacy plugin' }).click()

    await waitFor(() =>
      expect(requestGateway).toHaveBeenCalledWith(
        'plugins.manage',
        expect.objectContaining({ action: 'toggle', key: 'image_gen/legacy', enable: true })
      )
    )
  })

  it('renders keyless rows read-only (no name-addressed toggle RPC)', () => {
    // Name-addressed toggles flip every same-named plugin across category
    // dirs — pre-contract-v6 rows must never reach the RPC.
    $agentPlugins.set([
      {
        description: 'Returned by a pre-key backend',
        name: 'Legacy plugin',
        source: 'user',
        status: 'disabled',
        version: '0.20.0'
      }
    ])

    render(<PluginsTab profile={null} />)

    const toggle = screen.getByRole('switch', { name: 'Agent: Legacy plugin' })

    expect(toggle.hasAttribute('disabled') || toggle.getAttribute('aria-disabled') === 'true').toBe(true)

    toggle.click()

    expect(requestGateway).not.toHaveBeenCalledWith('plugins.manage', expect.objectContaining({ action: 'toggle' }))
  })

  it('appends the subdir fragment for multi-plugin repos', async () => {
    catalogRows = [
      catalogRow('nested-plugin', { repo: 'https://github.com/example/plugins-monorepo', subdir: 'nested-plugin' })
    ]
    render(<PluginsTab profile={null} />)
    await clickCatalogInstall('nested-plugin')

    await waitFor(() => {
      expect($pluginInstallRequest.get()?.repo).toBe('https://github.com/example/plugins-monorepo#nested-plugin')
    })
  })
})

describe('PluginsTab catalog UX', () => {
  beforeEach(() => {
    $agentPlugins.set([])
    $agentPluginsStatus.set('ready')
    closePluginInstallRequest()
    requestGateway.mockClear()
  })

  afterEach(cleanup)

  it('shows an Update chip when the catalog pin moved past the installed SHA', () => {
    $agentPlugins.set([
      {
        catalog_name: 'demo-weather',
        catalog_sha: 'b'.repeat(40),
        catalog_tier: 'community',
        description: '',
        installed_sha: 'a'.repeat(40),
        key: 'demo-weather',
        name: 'demo-weather',
        source: 'git',
        status: 'enabled',
        update_available: true,
        version: '1.0.0'
      }
    ])

    render(<PluginsTab profile={null} />)

    expect(screen.getByRole('button', { name: `Update to ${'b'.repeat(8)}` })).toBeTruthy()
  })

  it('re-pins through plugins.manage update when the chip is clicked', async () => {
    $agentPlugins.set([
      {
        catalog_name: 'demo-weather',
        catalog_sha: 'b'.repeat(40),
        catalog_tier: 'community',
        description: '',
        installed_sha: 'a'.repeat(40),
        key: 'demo-weather',
        name: 'demo-weather',
        source: 'git',
        status: 'enabled',
        update_available: true,
        version: '1.0.0'
      }
    ])
    requestGateway.mockResolvedValue({ ok: true, unchanged: false, plugins: [] } as never)

    render(<PluginsTab profile="workbot" />)

    screen.getByRole('button', { name: `Update to ${'b'.repeat(8)}` }).click()

    await waitFor(() =>
      expect(requestGateway).toHaveBeenCalledWith(
        'plugins.manage',
        expect.objectContaining({ action: 'update', name: 'demo-weather', profile: 'workbot' })
      )
    )
  })

  it('uninstalls through plugins.manage remove only after the confirm dialog is accepted', async () => {
    $agentPlugins.set([
      {
        description: '',
        key: 'demo-weather',
        name: 'demo-weather',
        source: 'git',
        status: 'enabled',
        version: '1.0.0'
      }
    ])
    requestGateway.mockResolvedValue({ ok: true, name: 'demo-weather', plugins: [] } as never)

    render(<PluginsTab profile="workbot" />)

    screen.getByRole('button', { name: 'Uninstall: demo-weather' }).click()

    // The click only asks; nothing is deleted until the destructive confirm is answered.
    await waitFor(() => expect($confirmRequest.get()?.title).toContain('demo-weather'))
    expect(requestGateway).not.toHaveBeenCalledWith('plugins.manage', expect.objectContaining({ action: 'remove' }))

    settleConfirm(true)

    await waitFor(() =>
      expect(requestGateway).toHaveBeenCalledWith(
        'plugins.manage',
        expect.objectContaining({ action: 'remove', name: 'demo-weather', profile: 'workbot' })
      )
    )
    await waitFor(() => expect(screen.queryByText('demo-weather')).toBeNull())
  })

  it('uninstalls a standalone desktop plugin through Electron only after the confirm dialog is accepted', async () => {
    $pluginRecords.set({
      clock: { id: 'clock', name: 'Clock', kind: 'disk', status: 'loaded', file: '/h/desktop-plugins/clock/plugin.js' }
    })
    uninstallDiskPlugin.mockClear()

    render(<PluginsTab profile={null} />)

    screen.getByRole('button', { name: 'Uninstall: Clock' }).click()

    await waitFor(() => expect($confirmRequest.get()?.title).toContain('Clock'))
    expect(uninstallDiskPlugin).not.toHaveBeenCalled()

    settleConfirm(true)

    await waitFor(() => expect(uninstallDiskPlugin).toHaveBeenCalledWith('clock'))
    // Nothing goes over the gateway: this half lives in this app, not the profile.
    expect(requestGateway).not.toHaveBeenCalledWith('plugins.manage', expect.objectContaining({ action: 'remove' }))
  })

  it('offers no desktop Uninstall for a bundled plugin or a unified package half', () => {
    $pluginRecords.set({
      bots: { id: 'bots', name: 'Bot Mode', kind: 'bundled', status: 'loaded' },
      media: { id: 'media', name: 'Media Studio', kind: 'disk', status: 'loaded', packageName: 'factr-media-studio' }
    })

    render(<PluginsTab profile={null} />)

    expect(screen.getByText('Bot Mode')).toBeTruthy()
    expect(screen.getByText('Media Studio')).toBeTruthy()
    expect(screen.queryByRole('button', { name: /^Uninstall:/ })).toBeNull()
  })

  it('offers no Uninstall for a pip-installed (entrypoint) agent plugin', () => {
    $agentPlugins.set([
      { description: '', key: 'demo-tool', name: 'demo-tool', source: 'entrypoint', status: 'enabled', version: '' }
    ])

    render(<PluginsTab profile={null} />)

    expect(screen.getByText('demo-tool')).toBeTruthy()
    expect(screen.queryByRole('button', { name: 'Uninstall: demo-tool' })).toBeNull()
  })

  it('refuses a catalog pick that is already installed and current', async () => {
    $agentPlugins.set([
      {
        catalog_name: 'demo-weather',
        description: '',
        installed_sha: 'a'.repeat(40),
        key: 'demo-weather',
        name: 'demo-weather',
        source: 'git',
        status: 'enabled',
        update_available: false,
        version: '1.0.0'
      }
    ])

    catalogRows = [catalogRow('demo-weather', { installed: true })]
    render(<PluginsTab profile={null} />)

    // An installed, current entry offers no install: its button is disabled and no modal opens.
    const row = await screen.findByTestId('catalog-row-demo-weather')

    expect((within(row).getByRole('button') as HTMLButtonElement).disabled).toBe(true)
    fireEvent.click(within(row).getByRole('button'))
    expect($pluginInstallRequest.get()).toBeNull()
  })

  it('still opens the modal for an installed pick when an update is available', async () => {
    $agentPlugins.set([
      {
        catalog_name: 'demo-weather',
        description: '',
        installed_sha: 'a'.repeat(40),
        key: 'demo-weather',
        name: 'demo-weather',
        source: 'git',
        status: 'enabled',
        update_available: true,
        version: '1.0.0'
      }
    ])

    catalogRows = [catalogRow('demo-weather', { installed: true, updateAvailable: true })]
    render(<PluginsTab profile={null} />)
    await clickCatalogInstall('demo-weather')

    await waitFor(() => expect($pluginInstallRequest.get()).not.toBeNull())
  })
})
