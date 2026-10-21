<p align="center"><img src="docs/assets/factr-mark.png" alt="Factr-I" width="160"></p>

# Factr-I

A desktop AI agent that runs on your computer. Chat with it, give it tasks, and let it work in your own folders, with
your approval for anything risky. Your chats, projects and settings stay on your machine.

Built by [Refactr](https://www.refactr.net), which deploys private AI infrastructure on hardware you
already own.

**[Download the latest release](https://github.com/Refactr-io/Factr-I/releases/latest)** for macOS, Windows or Linux.

## Download

| Your computer | Download from the latest release |
| --- | --- |
| Mac (Apple Silicon, M1 or later) | `Factr-I-<version>-mac-arm64.dmg` (or the `.zip`) |
| Windows 10/11 (64-bit) | `Factr-I-<version>-win-x64.exe` |
| Linux (64-bit) | `Factr-I-<version>-linux-x86_64.AppImage` |

Intel Macs are not supported. The installers are not signed with a paid developer certificate yet, so your system
shows a warning the first time. The steps below get you past it.

### Install on macOS

1. Open the `.dmg` and drag **Factr-I** into your **Applications** folder. Do this before opening it; an app run
   straight from the disk image cannot update or uninstall itself.
2. Open Factr-I from Applications. macOS says it "Not Opened". Close that message.
3. Go to **System Settings > Privacy & Security**, scroll down, click **Open Anyway** next to the Factr-I message,
   and confirm.

You only do this once.

### Install on Windows

1. Run the `.exe`.
2. If Windows SmartScreen warns you, click **More info**, then **Run anyway**.
3. To build the installer yourself, see [docs/WINDOWS-BUILD.md](docs/WINDOWS-BUILD.md).

### Install on Linux

```sh
chmod +x Factr-I-*.AppImage
./Factr-I-*.AppImage
```

On Ubuntu 24.04 you also need FUSE 2 (`sudo apt install libfuse2t64`), and you start the app with
`./Factr-I-*.AppImage --no-sandbox` because the system blocks the browser sandbox for AppImages.

## First run

1. Open Factr-I.
2. Choose how to connect a model:
   - **Sign in with ChatGPT** to use your ChatGPT account, or
   - **Add an API key** (OpenAI and other providers), or
   - **Use a local model** with Ollama, or **your own endpoint** (any OpenAI-compatible server).
3. Start a chat.

Your choice is saved. Factr-I uses the provider you picked and does not quietly switch to another one.

## What you can do

- **Chat and tasks.** Ask questions, write and edit code, and run commands in your folders.
- **Projects.** Create a project, add its folders, and keep its chats together in the sidebar.
- **Approvals you control.** Risky commands ask first. "Always allow" is tied to the folder, so a permission you
  give in one project does not carry to another.
- **Run history and usage.** The Agents page lists your runs, sessions, approvals and token usage.
- **Voice dictation.** Speak instead of typing. The first time you use it, the app downloads a small speech engine
  (about 200 MB) into your data folder. After that it works offline.
- **Bots.** Create separate bot profiles, each with its own settings and chats.
- **Updates.** The app checks this repository's releases and tells you when a new version is available. It never
  installs anything by itself.

## Your data

Everything stays on your computer:

- `~/.factr/` holds your settings, sign-ins, chats and projects. On Windows that is `%USERPROFILE%\.factr`.
- The in-app uninstall lists exactly what it will delete before it deletes anything.

## Running this for a business

Factr-I is the agent layer. [Refactr](https://www.refactr.net) builds the rest: Epsilon, a serving runtime
that makes open models fast enough to sit behind an agent on GPUs you already own, including consumer and
AMD cards.

If you want this running for a team, on your own hardware, with your data staying inside your network,
[talk to us about a pilot](https://www.refactr.net/request-demo).

## Troubleshooting

| Problem | What to do |
| --- | --- |
| macOS says the app cannot be opened | Follow the **Open Anyway** steps above. Make sure the app is in Applications. |
| Windows blocks the installer | **More info**, then **Run anyway**. |
| Linux AppImage will not start | Install FUSE 2 and start it with `--no-sandbox` (see above). |
| Dictation is slow the first time | It is downloading the speech engine once. The next use is quick. |
| A chat says no provider is connected | Open **Settings**, choose a provider and sign in or add a key. |

Found a problem? [Open an issue](https://github.com/Refactr-io/Factr-I/issues).

## Build it yourself

You do not need to build anything to use Factr-I. If you want to develop or verify it:

- Build steps for every platform, including Windows: [docs/BUILDING.md](docs/BUILDING.md)
- How releases are cut: [docs/RELEASING.md](docs/RELEASING.md)

| Path | What |
| --- | --- |
| `engine/` | The Rust engine (binary `factr`): the agent loop, tools, providers, approvals and the local gateway the app talks to. |
| `backend/` | The Python feature backend: bots, voice, plugins and settings helpers. |
| `desktop/` | The Electron and React desktop app. |

## License

Functional Source License 1.1 with an Apache 2.0 future license (FSL-1.1-ALv2), see `LICENSE`.
Free for internal use, education and research. Using it to build a product or service that
competes with Factr-I or with Refactr's deployment offering is not permitted. Each version
becomes Apache 2.0 two years after its release.

Third-party notices: `THIRD_PARTY_NOTICES.md`.
