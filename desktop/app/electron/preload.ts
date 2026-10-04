import { contextBridge, ipcRenderer, webFrame, webUtils } from 'electron'

import type { DesktopProfileRoute } from './desktop-profile'
import type { HudModifierApi, HudModifierStatus } from './hud-modifier-types'
import { customWindowControlsEnabled } from './window-controls'

// Which translucency the OS can back. Asked synchronously because the renderer
// needs it before its first paint, and answered by main because deciding it
// needs `os.release()` — a sandboxed preload may only require electron, events,
// timers and url, so importing node:os here throws before contextBridge runs
// and takes the ENTIRE bridge down with it (window.factrDesktop undefined =>
// "Desktop IPC bridge is unavailable"). No reply means no glass, which degrades
// to an ordinary opaque window rather than a page thinned over nothing.
const translucencySupport = ipcRenderer.sendSync('factr:translucency:support')
const hudWindowing = ipcRenderer.sendSync('factr:hud:windowing')
const hudNativeDrag = hudWindowing?.nativeDrag === true
const launchFlags = ipcRenderer.sendSync('factr:launch-flags')

contextBridge.exposeInMainWorld('factrDesktop', {
  glassSupported: translucencySupport?.glass === true,
  translucencySupported: translucencySupport?.translucency === true,
  // Launch-flag fact: the app was started with --local, so the renderer may
  // show the local-models surfaces. Static for the window's lifetime.
  localModelsEnabled: launchFlags?.localModels === true,
  // Launch-flag fact: guest onboarding is on for this launch
  // (FACTR_GUEST_ONBOARDING=1 or --guest-onboarding). Read-only; the same
  // decision is stamped onto every backend the app spawns.
  guestOnboardingEnabled: launchFlags?.guestOnboarding === true,
  // Launch-flag fact: skip the first-run film (FACTR_SKIP_INTRO=1 or
  // --skip-intro). Rehearsal aid for the guided chat behind it.
  skipIntro: launchFlags?.skipIntro === true,
  getConnection: (profile, opts) => ipcRenderer.invoke('factr:connection', profile, opts),
  // Registry-scoped backend resolution: { connectionId, profile } → descriptor.
  getConnectionFor: payload => ipcRenderer.invoke('factr:connection:for', payload),
  getProfileRoutes: profiles => ipcRenderer.invoke('factr:plugin-profile-routes', profiles),
  revalidateConnection: () => ipcRenderer.invoke('factr:connection:revalidate'),
  touchBackend: (profile, options) => ipcRenderer.invoke('factr:backend:touch', profile, options),
  getPoolLimits: () => ipcRenderer.invoke('factr:pool-limits:get'),
  setPoolLimits: limits => ipcRenderer.invoke('factr:pool-limits:set', limits),
  getGatewayWsUrl: profile => ipcRenderer.invoke('factr:gateway:ws-url', profile),
  // Registry-scoped fresh WS URL: { connectionId, profile } → result shape of
  // getGatewayWsUrl, minted against that connection's backend.
  getGatewayWsUrlFor: payload => ipcRenderer.invoke('factr:gateway:ws-url-for', payload),
  // Union agent roster across every registered connection.
  getAgentRoster: () => ipcRenderer.invoke('factr:agents:roster'),
  openSessionWindow: (sessionId, opts) => ipcRenderer.invoke('factr:window:openSession', sessionId, opts),
  openSessionInTerminal: (sessionId, opts) => ipcRenderer.invoke('factr:window:openInTerminal', sessionId, opts),
  openWindow: (options?: DesktopProfileRoute) => ipcRenderer.invoke('factr:window:openInstance', options),
  openBrowserWindow: tabId => ipcRenderer.invoke('factr:window:openBrowser', tabId),
  onBrowserPopoutClosed: callback => {
    const listener = (_event, tabId) => callback(tabId)
    ipcRenderer.on('factr:browser-popout:closed', listener)

    return () => ipcRenderer.removeListener('factr:browser-popout:closed', listener)
  },
  claimAmbientCue: key => ipcRenderer.invoke('factr:ambient:claim', key),
  windowControls: {
    custom: customWindowControlsEnabled(),
    minimize: () => ipcRenderer.send('factr:window-control', 'minimize'),
    toggleMaximize: () => ipcRenderer.send('factr:window-control', 'toggle-maximize'),
    close: () => ipcRenderer.send('factr:window-control', 'close')
  },
  wakeIndicator: {
    getState: () => ipcRenderer.invoke('factr:wake-indicator:get'),
    setState: state => ipcRenderer.send('factr:wake-indicator:set', state),
    onState: callback => {
      const listener = (_event, state) => callback(state)
      ipcRenderer.on('factr:wake-indicator:state', listener)

      return () => ipcRenderer.removeListener('factr:wake-indicator:state', listener)
    }
  },
  chatOnboarding: {
    grow: request => ipcRenderer.send('factr:chat-onboarding:grow', request),
    soloBoot: () => ipcRenderer.send('factr:chat-onboarding:solo-boot')
  },
  introReveal: {
    open: (payload?: { hideMain?: boolean }) => ipcRenderer.invoke('factr:intro-reveal:open', payload),
    close: (payload?: { showMain?: boolean }) => ipcRenderer.invoke('factr:intro-reveal:close', payload),
    skip: () => ipcRenderer.send('factr:intro-reveal:skip'),
    ready: () => ipcRenderer.send('factr:intro-reveal:ready'),
    onSkip: callback => {
      const listener = () => callback()

      ipcRenderer.on('factr:intro-reveal:skip', listener)

      return () => ipcRenderer.removeListener('factr:intro-reveal:skip', listener)
    },
    onClosed: callback => {
      const listener = () => callback()

      ipcRenderer.on('factr:intro-reveal:closed', listener)

      return () => ipcRenderer.removeListener('factr:intro-reveal:closed', listener)
    }
  },
  petOverlay: {
    // Main renderer → main process: window lifecycle + drag. `request` is
    // `{ bounds, screen }`; resolves with the screen bounds it actually used.
    open: request => ipcRenderer.invoke('factr:pet-overlay:open', request),
    close: () => ipcRenderer.invoke('factr:pet-overlay:close'),
    setBounds: bounds => ipcRenderer.send('factr:pet-overlay:set-bounds', bounds),
    setIgnoreMouse: ignore => ipcRenderer.send('factr:pet-overlay:ignore-mouse', ignore),
    // Flip the overlay focusable (and focus it) while the composer needs keys.
    setFocusable: focusable => ipcRenderer.send('factr:pet-overlay:set-focusable', focusable),
    // Main renderer → overlay (forwarded by main): push the latest pet state.
    pushState: payload => ipcRenderer.send('factr:pet-overlay:state', payload),
    // Overlay → main renderer (forwarded by main): pop back in / composer submit.
    control: payload => ipcRenderer.send('factr:pet-overlay:control', payload),
    // Overlay subscribes to state pushes.
    onState: callback => {
      const listener = (_event, payload) => callback(payload)
      ipcRenderer.on('factr:pet-overlay:state', listener)

      return () => ipcRenderer.removeListener('factr:pet-overlay:state', listener)
    },
    // Main renderer subscribes to overlay control messages.
    onControl: callback => {
      const listener = (_event, payload) => callback(payload)
      ipcRenderer.on('factr:pet-overlay:control', listener)

      return () => ipcRenderer.removeListener('factr:pet-overlay:control', listener)
    }
  },
  // HUD mode: the chrome-free floating chat. A full app renderer (own gateway)
  // sized as a floating bar, so it mounts the real composer. Main owns the
  // window; `onChanged` keeps every window's toggle truthful.
  hud: {
    nativeDrag: hudNativeDrag,
    windowing: {
      clientPlacement: hudWindowing?.clientPlacement !== false,
      controlDrag: hudWindowing?.controlDrag === true,
      nativeDrag: hudNativeDrag,
      solid: hudWindowing?.solid === true,
      workspaceTransfer: hudWindowing?.workspaceTransfer === true
    },
    open: request => ipcRenderer.invoke('factr:hud:open', request),
    close: () => ipcRenderer.invoke('factr:hud:close'),
    setIgnoreMouse: ignore => ipcRenderer.send('factr:hud:ignore-mouse', ignore),
    beginMove: () => ipcRenderer.send('factr:hud:begin-move'),
    endMove: () => ipcRenderer.send('factr:hud:end-move'),
    moveBy: delta => ipcRenderer.send('factr:hud:move-by', delta),
    setWorkspaceTransfer: transferring => ipcRenderer.send('factr:hud:workspace-transfer', transferring),
    setBounds: bounds => ipcRenderer.send('factr:hud:set-bounds', bounds),
    resetLayout: () => ipcRenderer.invoke('factr:hud:reset-layout'),
    // Whether the band covers the window below the bar. Main pairs it with the
    // user's translucency setting to decide the native frost (macOS vibrancy /
    // Windows 11 DWM backdrop) — see hudFrostFor.
    setFrost: showing => ipcRenderer.invoke('factr:hud:frost', showing),
    // The HUD tells main which session it is on; main hands that back to the
    // app window when the HUD closes, so the app can re-home onto it.
    setSession: sessionId => ipcRenderer.send('factr:hud:session', sessionId),
    onGoto: callback => {
      const listener = (_event, sessionId) => callback(sessionId)
      ipcRenderer.on('factr:hud:goto', listener)

      return () => ipcRenderer.removeListener('factr:hud:goto', listener)
    },
    onChanged: callback => {
      const listener = (_event, state) => callback(state)
      ipcRenderer.on('factr:hud:changed', listener)

      return () => ipcRenderer.removeListener('factr:hud:changed', listener)
    },
    // Linux only, and silent elsewhere: where the cursor is, in page
    // coordinates, or null when it has left the window. Stands in for the
    // mousemove that `setIgnoreMouseEvents(true, { forward: true })` delivers on
    // macOS and Windows but not here.
    onCursor: callback => {
      const listener = (_event, point) => callback(point)
      ipcRenderer.on('factr:hud:cursor', listener)

      return () => ipcRenderer.removeListener('factr:hud:cursor', listener)
    },
    // Main's game-overlay watch: whether a fullscreen app (a game) is under
    // the HUD, so the renderer can step back to the low-opacity overlay
    // treatment while one owns the screen.
    onGameOverlay: callback => {
      const listener = (_event, state) => callback(state)
      ipcRenderer.on('factr:hud:game-overlay', listener)

      return () => ipcRenderer.removeListener('factr:hud:game-overlay', listener)
    }
  },
  hudModifier: {
    getSettings: () => ipcRenderer.invoke('factr:hud-modifier:settings:get'),
    setEnabled: enabled => ipcRenderer.invoke('factr:hud-modifier:settings:set', enabled),
    openPermissionSettings: () => ipcRenderer.invoke('factr:hud-modifier:permission'),
    onStatus: callback => {
      const listener = (_event: Electron.IpcRendererEvent, status: HudModifierStatus) => callback(status)
      ipcRenderer.on('factr:hud-modifier:status', listener)

      return () => ipcRenderer.removeListener('factr:hud-modifier:status', listener)
    }
  } satisfies HudModifierApi,
  // macOS native screenshot gesture; captures require a main-issued request.
  screenshot:
    process.platform === 'darwin'
      ? {
          getSettings: () => ipcRenderer.invoke('factr:screenshot:settings:get'),
          setEnabled: enabled => ipcRenderer.invoke('factr:screenshot:settings:set', enabled),
          openPermissionSettings: kind => ipcRenderer.invoke('factr:screenshot:permission', kind),
          capture: requestId => ipcRenderer.invoke('factr:screenshot:capture', requestId),
          onStatus: callback => {
            const listener = (_event, status) => callback(status)
            ipcRenderer.on('factr:screenshot:status', listener)

            return () => ipcRenderer.removeListener('factr:screenshot:status', listener)
          },
          onRequest: callback => {
            const channel = 'factr:screenshot:request'
            const listener = (_event, requestId) => callback(requestId)

            if (ipcRenderer.listenerCount(channel) === 0) {
              ipcRenderer.send('factr:screenshot:subscribe', true)
            }

            ipcRenderer.on(channel, listener)

            return () => {
              ipcRenderer.removeListener(channel, listener)

              if (ipcRenderer.listenerCount(channel) === 0) {
                ipcRenderer.send('factr:screenshot:subscribe', false)
              }
            }
          }
        }
      : undefined,
  // Quick Entry: the global-hotkey mini composer window. Main owns the OS
  // shortcut + the persisted preference; the quick window only captures text
  // and hands it back, and the primary renderer submits it through the normal
  // prompt path.
  quickEntry: {
    getSettings: () => ipcRenderer.invoke('factr:quick-entry:settings:get'),
    setSettings: patch => ipcRenderer.invoke('factr:quick-entry:settings:set', patch),
    submit: payload => ipcRenderer.send('factr:quick-entry:submit', payload),
    dismiss: () => ipcRenderer.send('factr:quick-entry:dismiss'),
    // Primary renderer → main → quick window: gateway connection state + the
    // recent-session options the target picker offers. Main caches the latest
    // payload so a freshly spawned quick window starts from truth.
    pushState: payload => ipcRenderer.send('factr:quick-entry:state', payload),
    // Quick window subscribes to those pushes.
    onState: callback => {
      const listener = (_event, payload) => callback(payload)
      ipcRenderer.on('factr:quick-entry:state', listener)

      return () => ipcRenderer.removeListener('factr:quick-entry:state', listener)
    },
    // Main → primary renderer: a submit captured by the quick window.
    onSubmit: callback => {
      const listener = (_event, payload) => callback(payload)
      ipcRenderer.on('factr:quick-entry:submit', listener)

      return () => ipcRenderer.removeListener('factr:quick-entry:submit', listener)
    },
    // Main → quick window: you were just summoned (reset draft + refocus).
    onShown: callback => {
      const listener = () => callback()
      ipcRenderer.on('factr:quick-entry:shown', listener)

      return () => ipcRenderer.removeListener('factr:quick-entry:shown', listener)
    }
  },
  getBootProgress: () => ipcRenderer.invoke('factr:boot-progress:get'),
  getConnectionConfig: profile => ipcRenderer.invoke('factr:connection-config:get', profile),
  saveConnectionConfig: payload => ipcRenderer.invoke('factr:connection-config:save', payload),
  applyConnectionConfig: payload => ipcRenderer.invoke('factr:connection-config:apply', payload),
  testConnectionConfig: payload => ipcRenderer.invoke('factr:connection-config:test', payload),
  // Opt-in OS-keychain encryption for stored gateway secrets (default off —
  // see secret-storage-policy.ts). get never touches the OS keychain.
  getSecretStorageEncryption: () => ipcRenderer.invoke('factr:secret-storage:get'),
  setSecretStorageEncryption: (on: boolean) => ipcRenderer.invoke('factr:secret-storage:set', on),
  // v2 multi-connection registry: named agent sources (local / remote / cloud / ssh).
  connections: {
    list: () => ipcRenderer.invoke('factr:connections:list'),
    save: payload => ipcRenderer.invoke('factr:connections:save', payload),
    remove: id => ipcRenderer.invoke('factr:connections:remove', id),
    setPrimary: id => ipcRenderer.invoke('factr:connections:set-primary', id),
    setLaunchMode: mode => ipcRenderer.invoke('factr:connections:set-launch-mode', mode),
    setLastUsed: id => ipcRenderer.invoke('factr:connections:set-last-used', id),
    test: id => ipcRenderer.invoke('factr:connections:test', id),
    updateManaged: id => ipcRenderer.invoke('factr:connections:update-managed', id),
    // Fan out `factr update` to every eligible registered connection.
    // Optional excludeIds skips rows the caller updates through another path.
    updateAll: options => ipcRenderer.invoke('factr:connections:update-all', options),
    // Registry lifecycle push (main → renderer): a connection was removed or
    // materially edited, so secondaries scoped to it must be disposed (and,
    // for edits, re-dialed at the new target).
    onChanged: callback => {
      const listener = (_event, payload) => callback(payload)
      ipcRenderer.on('factr:connections:changed', listener)

      return () => ipcRenderer.removeListener('factr:connections:changed', listener)
    }
  },
  sshConfigHosts: () => ipcRenderer.invoke('factr:ssh-config:hosts'),
  sshResolveHost: host => ipcRenderer.invoke('factr:ssh-config:resolve', host),
  probeConnectionConfig: remoteUrl => ipcRenderer.invoke('factr:connection-config:probe', remoteUrl),
  oauthLoginConnectionConfig: remoteUrl => ipcRenderer.invoke('factr:connection-config:oauth-login', remoteUrl),
  oauthLogoutConnectionConfig: remoteUrl => ipcRenderer.invoke('factr:connection-config:oauth-logout', remoteUrl),
  profile: {
    getDefault: () => ipcRenderer.invoke('factr:profile:default:get'),
    setDefault: (route: DesktopProfileRoute) => ipcRenderer.invoke('factr:profile:default:set', route),
    onDefaultChanged: (callback: (route: DesktopProfileRoute | null) => void) => {
      const listener = (_event: Electron.IpcRendererEvent, route: DesktopProfileRoute | null) => callback(route)
      ipcRenderer.on('factr:profile:default:changed', listener)

      return () => ipcRenderer.removeListener('factr:profile:default:changed', listener)
    },
    get: () => ipcRenderer.invoke('factr:profile:get'),
    remember: name => ipcRenderer.invoke('factr:profile:remember', name),
    set: name => ipcRenderer.invoke('factr:profile:set', name)
  },
  api: request => ipcRenderer.invoke('factr:api', request),
  notify: payload => ipcRenderer.invoke('factr:notify', payload),
  requestMicrophoneAccess: () => ipcRenderer.invoke('factr:requestMicrophoneAccess'),
  readWindowBelow: () => ipcRenderer.invoke('factr:window:readBelow'),
  readFileDataUrl: filePath => ipcRenderer.invoke('factr:readFileDataUrl', filePath),
  readFileDataUrlForAttach: filePath => ipcRenderer.invoke('factr:readFileDataUrlForAttach', filePath),
  dataUrlReadMax: {
    get: () => ipcRenderer.invoke('factr:data-url-read-max:get'),
    set: maxMb => ipcRenderer.invoke('factr:data-url-read-max:set', maxMb)
  },
  readFileText: filePath => ipcRenderer.invoke('factr:readFileText', filePath),
  readPluginSource: (filePath: string) => ipcRenderer.invoke('factr:readPluginSource', filePath),
  selectPaths: options => ipcRenderer.invoke('factr:selectPaths', options),
  selectSavePath: options => ipcRenderer.invoke('factr:selectSavePath', options),
  writeClipboard: text => ipcRenderer.invoke('factr:writeClipboard', text),
  readClipboard: () => ipcRenderer.invoke('factr:readClipboard'),
  saveGatewayFile: payload => ipcRenderer.invoke('factr:saveGatewayFile', payload),
  saveImageFromUrl: url => ipcRenderer.invoke('factr:saveImageFromUrl', url),
  contextMenuEdit: command => ipcRenderer.invoke('factr:context-menu:edit', command),
  contextMenuCopyImage: () => ipcRenderer.invoke('factr:context-menu:copy-image'),
  contextMenuSpellcheck: action => ipcRenderer.invoke('factr:context-menu:spellcheck', action),
  contextMenuGuestAddWord: payload => ipcRenderer.invoke('factr:context-menu:guest-add-word', payload),
  onContextMenuSpellcheck: callback => {
    const listener = (_event, payload) => callback(payload)
    ipcRenderer.on('factr:context-menu-spellcheck', listener)

    return () => ipcRenderer.removeListener('factr:context-menu-spellcheck', listener)
  },
  saveImageBuffer: (data, ext, name) => ipcRenderer.invoke('factr:saveImageBuffer', { data, ext, name }),
  capturePreview: payload => ipcRenderer.invoke('factr:capturePreview', payload),
  savePastedText: text => ipcRenderer.invoke('factr:savePastedText', { text }),
  saveClipboardImage: () => ipcRenderer.invoke('factr:saveClipboardImage'),
  getPathForFile: file => {
    try {
      return webUtils.getPathForFile(file) || ''
    } catch {
      return ''
    }
  },
  normalizePreviewTarget: (target, baseDir) => ipcRenderer.invoke('factr:normalizePreviewTarget', target, baseDir),
  watchPreviewFile: url => ipcRenderer.invoke('factr:watchPreviewFile', url),
  watchDirectory: dir => ipcRenderer.invoke('factr:watchDirectory', dir),
  stopPreviewFileWatch: id => ipcRenderer.invoke('factr:stopPreviewFileWatch', id),
  setActiveWork: payload => ipcRenderer.send('factr:active-work', payload),
  setTitleBarTheme: payload => ipcRenderer.send('factr:titlebar-theme', payload),
  setNativeTheme: mode => ipcRenderer.send('factr:native-theme', mode),
  setTranslucency: payload => ipcRenderer.send('factr:translucency', payload),
  setKeepAwake: on => ipcRenderer.send('factr:keep-awake', on),
  minimizeToTray: {
    get: () => ipcRenderer.invoke('factr:minimize-to-tray:get'),
    set: on => ipcRenderer.invoke('factr:minimize-to-tray:set', on),
    onChanged: callback => {
      const listener = (_event, status) => callback(status)
      ipcRenderer.on('factr:minimize-to-tray:changed', listener)

      return () => ipcRenderer.removeListener('factr:minimize-to-tray:changed', listener)
    }
  },
  setDisableF12: blocked => ipcRenderer.send('factr:devtools:disable-f12', blocked),
  setPreviewShortcutActive: active => ipcRenderer.send('factr:previewShortcutActive', Boolean(active)),
  openExternal: url => ipcRenderer.invoke('factr:openExternal', url),
  openThirdPartyNotices: () => ipcRenderer.invoke('factr:openThirdPartyNotices'),
  mcpOauth: {
    // One-shot loopback listener for MCP OAuth against remote backends: bind
    // on this machine, hand redirectUri to mcp.servers.oauth.start, then wait
    // for the provider redirect and relay code/state via oauth.callback.
    listen: () => ipcRenderer.invoke('factr:mcp-oauth:listen'),
    wait: (id, timeoutMs) => ipcRenderer.invoke('factr:mcp-oauth:wait', id, timeoutMs),
    cancel: id => ipcRenderer.invoke('factr:mcp-oauth:cancel', id)
  },
  openPreviewInBrowser: url => ipcRenderer.invoke('factr:openPreviewInBrowser', url),
  reachPreviewUrl: url => ipcRenderer.invoke('factr:preview:reach', url),
  setActiveConnectionRoute: route => ipcRenderer.send('factr:connection:active-route', route),
  fetchLinkTitle: url => ipcRenderer.invoke('factr:fetchLinkTitle', url),
  resolveFavicon: url => ipcRenderer.invoke('factr:resolveFavicon', url),
  sanitizeWorkspaceCwd: cwd => ipcRenderer.invoke('factr:workspace:sanitize', cwd),
  settings: {
    getDefaultProjectDir: () => ipcRenderer.invoke('factr:setting:defaultProjectDir:get'),
    setDefaultProjectDir: dir => ipcRenderer.invoke('factr:setting:defaultProjectDir:set', dir),
    pickDefaultProjectDir: () => ipcRenderer.invoke('factr:setting:defaultProjectDir:pick')
  },
  zoom: {
    // Current zoom of this window, as { level, percent }.
    get: () => ipcRenderer.invoke('factr:zoom:get'),
    // Synchronous zoom factor (1 = 100%). Coordinate math needs it in the
    // same tick as the event it converts, so no IPC round-trip here.
    factor: () => webFrame.getZoomFactor(),
    setPercent: percent => ipcRenderer.send('factr:zoom:set-percent', percent),
    // Fires on every zoom change, including the Ctrl/Cmd +/-/0 shortcuts,
    // so the settings UI can stay in sync with the keyboard.
    onChanged: callback => {
      const listener = (_event, payload) => callback(payload)
      ipcRenderer.on('factr:zoom:changed', listener)

      return () => ipcRenderer.removeListener('factr:zoom:changed', listener)
    }
  },
  revealLogs: () => ipcRenderer.invoke('factr:logs:reveal'),
  getRecentLogs: () => ipcRenderer.invoke('factr:logs:recent'),
  // Fire-and-forget: persists a renderer error-boundary catch (with component
  // stack) to desktop.log so crashes survive the window (#79428).
  reportRendererError: report => ipcRenderer.send('factr:logs:renderer-error', report),
  readDir: dirPath => ipcRenderer.invoke('factr:fs:readDir', dirPath),
  gitRoot: startPath => ipcRenderer.invoke('factr:fs:gitRoot', startPath),
  revealPath: targetPath => ipcRenderer.invoke('factr:fs:reveal', targetPath),
  openDir: dirPath => ipcRenderer.invoke('factr:fs:openDir', dirPath),
  desktopPluginsRoot: () => ipcRenderer.invoke('factr:fs:desktopPluginsRoot'),
  reconcileDesktopPlugins: () => ipcRenderer.invoke('factr:fs:reconcileDesktopPlugins'),
  logsRoot: () => ipcRenderer.invoke('factr:fs:logsRoot'),
  renamePath: (targetPath, newName) => ipcRenderer.invoke('factr:fs:rename', targetPath, newName),
  writeTextFile: (filePath, content) => ipcRenderer.invoke('factr:fs:writeText', filePath, content),
  trashPath: targetPath => ipcRenderer.invoke('factr:fs:trash', targetPath),
  git: {
    worktreeList: repoPath => ipcRenderer.invoke('factr:git:worktreeList', repoPath),
    worktreeAdd: (repoPath, options) => ipcRenderer.invoke('factr:git:worktreeAdd', repoPath, options),
    worktreeRemove: (repoPath, worktreePath, options) =>
      ipcRenderer.invoke('factr:git:worktreeRemove', repoPath, worktreePath, options),
    branchSwitch: (repoPath, branch) => ipcRenderer.invoke('factr:git:branchSwitch', repoPath, branch),
    branchList: repoPath => ipcRenderer.invoke('factr:git:branchList', repoPath),
    baseBranchList: repoPath => ipcRenderer.invoke('factr:git:baseBranchList', repoPath),
    repoStatus: repoPath => ipcRenderer.invoke('factr:git:repoStatus', repoPath),
    fileDiff: (repoPath, filePath) => ipcRenderer.invoke('factr:git:fileDiff', repoPath, filePath),
    scanRepos: (roots, options) => ipcRenderer.invoke('factr:git:scanRepos', roots, options),
    review: {
      list: (repoPath, scope, baseRef) => ipcRenderer.invoke('factr:git:review:list', repoPath, scope, baseRef),
      diff: (repoPath, filePath, scope, baseRef, staged) =>
        ipcRenderer.invoke('factr:git:review:diff', repoPath, filePath, scope, baseRef, staged),
      stage: (repoPath, filePath) => ipcRenderer.invoke('factr:git:review:stage', repoPath, filePath),
      unstage: (repoPath, filePath) => ipcRenderer.invoke('factr:git:review:unstage', repoPath, filePath),
      revert: (repoPath, filePath) => ipcRenderer.invoke('factr:git:review:revert', repoPath, filePath),
      revParse: (repoPath, ref) => ipcRenderer.invoke('factr:git:review:revParse', repoPath, ref),
      commit: (repoPath, message, push) => ipcRenderer.invoke('factr:git:review:commit', repoPath, message, push),
      commitContext: repoPath => ipcRenderer.invoke('factr:git:review:commitContext', repoPath),
      push: repoPath => ipcRenderer.invoke('factr:git:review:push', repoPath),
      shipInfo: repoPath => ipcRenderer.invoke('factr:git:review:shipInfo', repoPath),
      prList: (repoPath, branches, numbers) =>
        ipcRenderer.invoke('factr:git:review:prList', repoPath, branches, numbers),
      createPr: repoPath => ipcRenderer.invoke('factr:git:review:createPr', repoPath)
    }
  },
  terminal: {
    attach: id => ipcRenderer.invoke('factr:terminal:attach', id),
    cwd: id => ipcRenderer.invoke('factr:terminal:cwd', id),
    dispose: id => ipcRenderer.invoke('factr:terminal:dispose', id),
    resize: (id, size) => ipcRenderer.invoke('factr:terminal:resize', id, size),
    start: options => ipcRenderer.invoke('factr:terminal:start', options),
    write: (id, data) => ipcRenderer.invoke('factr:terminal:write', id, data),
    onData: (id, callback) => {
      const channel = `factr:terminal:${id}:data`
      const listener = (_event, payload) => callback(payload)
      ipcRenderer.on(channel, listener)

      return () => ipcRenderer.removeListener(channel, listener)
    },
    onExit: (id, callback) => {
      const channel = `factr:terminal:${id}:exit`
      const listener = (_event, payload) => callback(payload)
      ipcRenderer.on(channel, listener)

      return () => ipcRenderer.removeListener(channel, listener)
    }
  },
  onClosePreviewRequested: callback => {
    const listener = () => callback()
    ipcRenderer.on('factr:close-preview-requested', listener)

    return () => ipcRenderer.removeListener('factr:close-preview-requested', listener)
  },
  onPreviewNav: callback => {
    const listener = (_event, command) => callback(command)
    ipcRenderer.on('factr:preview-nav', listener)

    return () => ipcRenderer.removeListener('factr:preview-nav', listener)
  },
  onOpenFolderRequested: callback => {
    const listener = () => callback()
    ipcRenderer.on('factr:open-folder-requested', listener)

    return () => ipcRenderer.removeListener('factr:open-folder-requested', listener)
  },
  onOpenUpdatesRequested: callback => {
    const listener = () => callback()
    ipcRenderer.on('factr:open-updates', listener)

    return () => ipcRenderer.removeListener('factr:open-updates', listener)
  },
  onDeepLink: callback => {
    const listener = (_event, payload) => callback(payload)
    ipcRenderer.on('factr:deep-link', listener)

    return () => ipcRenderer.removeListener('factr:deep-link', listener)
  },
  signalDeepLinkReady: () => ipcRenderer.invoke('factr:deep-link-ready'),
  probePluginRepo: payload => ipcRenderer.invoke('factr:plugin:probe', payload),
  installDesktopPlugin: payload => ipcRenderer.invoke('factr:plugin:installDesktop', payload),
  removeDesktopPlugin: payload => ipcRenderer.invoke('factr:plugin:removeDesktop', payload),
  onWindowStateChanged: callback => {
    const listener = (_event, payload) => callback(payload)
    ipcRenderer.on('factr:window-state-changed', listener)

    return () => ipcRenderer.removeListener('factr:window-state-changed', listener)
  },
  onFocusSession: callback => {
    const listener = (_event, sessionId) => callback(sessionId)
    ipcRenderer.on('factr:focus-session', listener)

    return () => ipcRenderer.removeListener('factr:focus-session', listener)
  },
  onNotificationAction: callback => {
    const listener = (_event, payload) => callback(payload)
    ipcRenderer.on('factr:notification-action', listener)

    return () => ipcRenderer.removeListener('factr:notification-action', listener)
  },
  onNotificationActivate: callback => {
    const listener = (_event, payload) => callback(payload)
    ipcRenderer.on('factr:notification-activate', listener)

    return () => ipcRenderer.removeListener('factr:notification-activate', listener)
  },
  onPreviewFileChanged: callback => {
    const listener = (_event, payload) => callback(payload)
    ipcRenderer.on('factr:preview-file-changed', listener)

    return () => ipcRenderer.removeListener('factr:preview-file-changed', listener)
  },
  onBackendExit: callback => {
    const listener = (_event, payload) => callback(payload)
    ipcRenderer.on('factr:backend-exit', listener)

    return () => ipcRenderer.removeListener('factr:backend-exit', listener)
  },
  // Cooperative pool retirement (main → renderer): the pooled backend under
  // `poolKey` is being stopped for a foreground open. Park that scope; do not
  // redial into the slot it vacated.
  onPoolBackendRetiring: callback => {
    const listener = (_event, payload) => callback(payload)
    ipcRenderer.on('factr:pool:retiring', listener)

    return () => ipcRenderer.removeListener('factr:pool:retiring', listener)
  },
  // Soft gateway-mode apply finished tearing down the primary backend. Renderer
  // should wipe session lists + re-dial without a window reload.
  onConnectionApplied: callback => {
    const listener = () => callback()
    ipcRenderer.on('factr:connection:applied', listener)

    return () => ipcRenderer.removeListener('factr:connection:applied', listener)
  },
  onPowerResume: callback => {
    const listener = () => callback()
    ipcRenderer.on('factr:power-resume', listener)

    return () => ipcRenderer.removeListener('factr:power-resume', listener)
  },
  // AC ↔ battery transitions; renderers slow their backstop polls on battery.
  getOnBattery: () => ipcRenderer.invoke('factr:power-battery:get'),
  onBatteryChanged: callback => {
    const listener = (_event, onBattery) => callback(Boolean(onBattery))
    ipcRenderer.on('factr:power-battery', listener)

    return () => ipcRenderer.removeListener('factr:power-battery', listener)
  },
  onBootProgress: callback => {
    const listener = (_event, payload) => callback(payload)
    ipcRenderer.on('factr:boot-progress', listener)

    return () => ipcRenderer.removeListener('factr:boot-progress', listener)
  },
  // First-launch bootstrap progress -- emitted by the install.ps1 stage
  // runner in main.ts (apps/desktop/electron/bootstrap-runner.ts).
  // Renderer's install overlay subscribes to live events and queries the
  // current snapshot via getBootstrapState() to recover after a devtools
  // reload mid-bootstrap.
  getBootstrapState: () => ipcRenderer.invoke('factr:bootstrap:get'),
  continueBootstrapLocal: () => ipcRenderer.invoke('factr:bootstrap:continue-local'),
  recycleBackend: profile => ipcRenderer.invoke('factr:backend:recycle', profile),
  resetBootstrap: () => ipcRenderer.invoke('factr:bootstrap:reset'),
  repairBootstrap: () => ipcRenderer.invoke('factr:bootstrap:repair'),
  cancelBootstrap: () => ipcRenderer.invoke('factr:bootstrap:cancel'),
  onBootstrapEvent: callback => {
    const listener = (_event, payload) => callback(payload)
    ipcRenderer.on('factr:bootstrap:event', listener)

    return () => ipcRenderer.removeListener('factr:bootstrap:event', listener)
  },
  getVersion: () => ipcRenderer.invoke('factr:version'),
  relaunchApp: () => ipcRenderer.invoke('factr:app:relaunch'),
  getMachineProfile: () => ipcRenderer.invoke('factr:machine:profile'),
  getRemoteDisplayReason: () => ipcRenderer.invoke('factr:get-remote-display-reason'),
  uninstall: {
    summary: () => ipcRenderer.invoke('factr:uninstall:summary'),
    run: mode => ipcRenderer.invoke('factr:uninstall:run', { mode })
  },
  updates: {
    check: opts => ipcRenderer.invoke('factr:updates:check', opts),
    apply: opts => ipcRenderer.invoke('factr:updates:apply', opts)
  },
  // Find-in-page (Ctrl/Cmd+F): delegates to Electron's
  // webContents.findInPage on the IPC sender's window so a Cmd+F pressed
  // in a secondary session window searches THAT window, not the primary.
  // `onFoundInPage` returns the unsubscribe fn; the renderer wires it via
  // `initFindInPageListener` in store/find-in-page.ts and tears it down
  // when the FindBar unmounts.
  findInPage: (query, options) => ipcRenderer.invoke('factr:find-in-page', query, options),
  stopFindInPage: () => ipcRenderer.invoke('factr:stop-find-in-page'),
  onFoundInPage: callback => {
    const listener = (_event, result) => callback(result)
    ipcRenderer.on('factr:found-in-page', listener)

    return () => ipcRenderer.removeListener('factr:found-in-page', listener)
  },
  // Main-process `before-input-event` forwards Ctrl/Cmd+F here so renderer
  // can open the FindBar even when the GTK compositor has already grabbed
  // the chord at the windowing layer (#81727).
  onOpenFindBarRequested: callback => {
    const listener = () => callback()
    ipcRenderer.on('factr:open-find-bar', listener)

    return () => ipcRenderer.removeListener('factr:open-find-bar', listener)
  }
})
