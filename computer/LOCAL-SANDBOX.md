# Local bot sandbox

When the packaged Rust runtime routes a managed bot to a host engine, Desktop confines that whole engine before it starts. Native file tools, shell children, Python features and browser children inherit the boundary. A failed sandbox launch produces `local-sandbox-unavailable`; Desktop does not retry without confinement. Remote profiles and the default chat keep their existing routes.

## Storage and routing

For `<config>/profiles/<bot>`:

| Data | Location |
| --- | --- |
| Engine sessions, memory and engine settings | `engine/` |
| Working files | `workspace/` |
| Browser cookies and site storage | `browser-profile/` |
| Home, caches, downloaded browser and lazy packages | `.local-computer/` |
| Durable courier mail | `bot_relay/` |
| Cron jobs and output | `cron/` |

The engine sees a private single-profile configuration root through checked directory links. Desktop remains the courier between engines; it does not share another bot's folder. Profile aliases and state-directory links outside the bot are rejected. A `.computer` marker prevents a host engine from opening a Docker-owned profile, including while Docker is down. Explicit Docker Stop remains stopped across reconnects until explicit Start.

Old host bots previously used the shared engine store. This change retains that store and starts the private store; it does not import or delete old transcripts. A Docker computer and a local sandbox use the same profile-local `engine/` layout, but switching requires stopping its current owner first. Automatic Docker-to-local migration is intentionally refused while a `.computer` marker exists.

No host OAuth store is copied for the local fallback. Use the bot's own provider sign-in or its profile's existing API-key configuration. Copying single-use OAuth refresh credentials into independent stores can invalidate another session. This run used synthetic credentials only and did not test real sign-in.

## Platform boundaries

| Platform | Mechanism and requirements | Verified here |
| --- | --- | --- |
| macOS | Built-in `sandbox-exec` Seatbelt policy, deny by default, runtime/system reads, writes only in the bot and its temporary directory, own-sandbox signals. Host keychain and pasteboard services denied. | Real engines, native file/shell tools, sibling signal denial, symlink denial, feature Python, queued handoff/reply, restart, scheduled turn and output. Real headless Chromium with private persistent cookies and a disposable outside-app launch probe. |
| Linux | Existing `/usr/bin/bwrap` or `/bin/bwrap`, working unprivileged user namespaces. Private user/PID/IPC/UTS namespaces, private `/tmp`, read-only system/runtime mounts, only the bot and its temporary directory writable. | Real non-root bubblewrap filesystem, symlink, private-temp and PID boundary inside a temporary Linux container, plus a real engine native shell turn and bundled Python feature RPC. |
| Windows | Bundled Python standard-library launcher, restricted low-integrity token, per-bot protected ACLs, private desktop, kill-on-close Job Object without breakaway. Engine is created suspended and resumed only after job assignment. | Manifest and lifecycle/error-order fakes only. Real WinAPI/ACL/browser/child cleanup remains manual QA. |

Linux without Docker and without usable bubblewrap currently fails closed. Bubblewrap is not yet bundled by the release pipeline. There is no Landlock-only retry: older Landlock ABIs do not cover all Unix socket interactions. Administrators may also disable user namespaces. This is an explicit gap in the install-only-Factr-I goal on those Linux systems.

On macOS, the engine's nested Seatbelt REPL is unavailable, so local bot REPL is disabled. Shell and native files remain available. Chromium uses the outer Seatbelt boundary with its nested sandbox disabled, its own profile, mock keychain and basic password storage. Its cookies survive restart; this is not the host keychain's encryption. First use may download a browser into the private bot cache using the existing permitted lazy-install flow. The fixture used an existing read-only Chromium bundle; the packaged cold-download flow still needs GUI QA.

## This tier shares the host

These are local OS sandboxes, not virtual machines. Internet and IP loopback remain shared so local model servers, the feature backend and browser CDP can work. Other host services reachable over IP remain reachable subject to their own authentication. System/runtime files are readable; metadata restrictions differ by OS. Do not treat this tier as a network or full desktop-security boundary.

The Screen/noVNC desktop and human takeover remain the Linux Docker-computer feature. A local macOS/Windows bot has private browser storage, not an independently streamed operating-system desktop. Use a Docker computer for that UI.

Bots with enabled schedules start at Desktop launch and are excluded from host idle/LRU retirement. They still count toward the existing host pool cap, three by default. If schedules fill every slot, another open waits and can reach the existing bounded capacity error; raise the existing limit when needed. Host schedules require Desktop to remain running. Running Docker computers with enabled schedules keep their existing app-quit behavior. Bot-to-bot courier delivery requires Desktop.

## Verification

Use fresh HOME, FACTR_HOME, FACTR_CONFIG_HOME and Electron userData directories. Never point the tests at a real profile.

```sh
cd desktop/app
../node_modules/.bin/vitest run --project electron electron/bot-local-sandbox.test.ts electron/bot-computers.test.ts electron/bot-docker-setup.test.ts electron/pool-retire.test.ts
cd ../..
desktop/node_modules/.bin/tsx engine/crates/factr-gateway/e2e/bot-local-sandbox.mts
BOTS_TEST_CHROMIUM='/path/to/Google Chrome for Testing.app' desktop/node_modules/.bin/tsx engine/crates/factr-gateway/e2e/bot-local-browser.mts
computer/tests/test_local_sandbox_linux.sh
```

The Linux harness checks `docker info`, uses one `bots-` container and removes it on exit. Its nested-namespace test relaxes the *outer test container's* seccomp/AppArmor/system-path masking; these options are not bot computer run flags. Windows and packaged-app procedures are in `NEEDS-GUI-QA.md`.

Implementation references: [Windows restricted tokens](https://learn.microsoft.com/en-us/windows/win32/secauthz/restricted-tokens), [Landlock ABI boundaries](https://docs.kernel.org/userspace-api/landlock.html), [Chromium macOS temporary directory](https://chromium.googlesource.com/chromium/src/+/main/base/files/file_util_apple.mm).
