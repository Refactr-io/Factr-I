# Factr CLI Reference

Live sources when anything looks stale: `factr --help`, `factr <command> --help`,
https://github.com/Refactr-io/Factr-I

### Global Flags

```
factr [flags] [command]        (no subcommand = interactive chat)

  --version, -V             Show version
  -z, --oneshot PROMPT      One-shot: print ONLY the final response (for scripts/pipes)
  -m MODEL  --provider P    Model/provider override for this invocation
  -t, --toolsets LIST       Comma-separated toolsets for this invocation
  --resume, -r SESSION      Resume session by ID or title
  --continue, -c [NAME]     Resume by name, or most recent session
  --worktree, -w            Isolated git worktree mode (parallel agents)
  --skills, -s SKILL        Preload skills (comma-separate or repeat)
  --profile, -p NAME        Use a named profile
  --yolo                    Skip dangerous command approval
  --tui / --cli             Force the Ink TUI / classic REPL
  --ignore-rules            Skip AGENTS.md/SOUL.md/memory/skill injection
  --safe-mode               Disable ALL customizations (troubleshooting)
  --pass-session-id         Include session ID in system prompt
```

### Chat

```
factr chat [flags]
  -q, --query TEXT          Single query, non-interactive
  --image PATH              Attach a local image to a single query
  -Q, --quiet               Suppress banner, spinner, tool previews
  --checkpoints             Enable filesystem checkpoints (/rollback)
  --max-turns N             Cap tool-calling iterations
  --source TAG              Session source tag (default: cli)
```
(plus the global flags above)

### Configuration

```
factr setup [section]      Wizard (model|tts|terminal|gateway|tools|agent)
factr model                Interactive model/provider picker
factr fallback [add|remove|list]  Fallback provider chain
factr config [show|edit|get|set|unset|path|env-path|check|migrate]
factr login / logout       OAuth sign-in / clear stored auth
factr doctor [--fix]       Check dependencies and config
factr status [--all]       Component status
```

### Tools & Skills

```
factr tools [list|enable NAME|disable NAME]   Per-platform toolsets (curses UI with no args)

factr skills list|browse|search QUERY|inspect ID
factr skills install ID    Hub identifier OR a direct https://…/SKILL.md URL
factr skills config        Enable/disable skills per platform
factr skills check|update|uninstall|publish PATH
factr skills tap add REPO  Add a GitHub repo as a skill source
factr bundles              Skill bundles (one /<name> alias loads several skills)
```

### MCP Servers

```
factr mcp add NAME (--url or --command) | remove | list | test NAME
factr mcp catalog | install NAME     Curated catalog install
factr mcp configure NAME             Toggle tool selection
factr mcp serve                      Run Factr as an MCP server
```
Details (transport, tool discovery, catalog): `references/native-mcp.md`.

### Gateway (Messaging Platforms)

```
factr gateway run|install|start|stop|restart|status|setup
```

20+ platforms: Telegram, Discord, Slack, WhatsApp (Baileys + Business Cloud API), iMessage (Photon — `factr photon setup`), Signal, Email, SMS, Matrix, Mattermost, Teams, LINE, SimpleX, ntfy, Google Chat, Home Assistant, DingTalk, Feishu, WeCom, Weixin, API Server, Webhooks. Open WebUI connects via the API Server adapter. Most adapters ship under `plugins/platforms/`.
Docs: https://github.com/Refactr-io/Factr-I

### Sessions

```
factr sessions list|browse|rename ID TITLE|delete ID|export OUT|prune|stats
```

### Cron / Webhooks

```
factr cron list|create SCHED|edit ID|pause|resume|run ID|remove|status
    Schedules: '30m', 'every 2h', '0 9 * * *', ISO timestamp
factr webhook subscribe NAME|list|remove NAME|test NAME
```
Webhook payloads/routes: `references/webhooks.md`.

### Profiles

```
factr profile list|create NAME (--clone|--clone-all|--clone-from)|use|show|delete
factr profile rename A B | alias NAME | export NAME | import FILE
factr profile migrate-identity A B   Retry a completed rename's session/routing identity migration
```

### Credentials & Pools

```
factr auth                 Interactive credential manager
factr auth add [PROVIDER]  Add OAuth or API-key credential (openai-codex, qwen-oauth, …)
factr auth list|remove P IDX|reset PROVIDER|status
```
Multiple credentials per provider form a pool that rotates automatically and skips exhausted keys.

### Other

```
factr desktop / gui        Native desktop app
factr dashboard            Web admin panel + embedded chat (--stop / --status)
factr proxy                OpenAI-compatible local proxy backed by an OAuth provider
factr kanban <verb>        Multi-agent work-queue board
factr project              Named multi-folder workspaces
factr skin list|use|set    Switch/tweak skins (see references/themes.md)
factr pets <verb>          Pet mascots (see references/petdex.md)
factr memory setup|status|off|reset   Memory provider
factr secrets bitwarden|onepassword   External secret stores
factr moa                  Mixture-of-Agents slots
factr hooks / security / backup / import / checkpoints / console
factr logs [-f] [errors]   View agent/error logs
factr send                 One-off message through a gateway platform
factr pairing / plugins / insights / journey / computer-use
factr acp                  ACP server (IDE integration)
factr completion bash|zsh|fish
factr update / uninstall / claw migrate
```

Plugin- and provider-supplied subcommands (e.g. `factr photon setup`) only appear once their plugin is installed/active.

### Where to Find Things

| Looking for... | Location |
|---|---|
| Config options | `factr config edit` · [Configuration docs](https://github.com/Refactr-io/Factr-I) |
| Tools / toolsets | `factr tools list` · [Tools reference](https://github.com/Refactr-io/Factr-I) |
| Skills catalog | `factr skills browse` · [Skills catalog](https://github.com/Refactr-io/Factr-I) |
| Provider setup | `factr model` · [Providers guide](https://github.com/Refactr-io/Factr-I) |
| Env variables | `factr config env-path` · [Env vars reference](https://github.com/Refactr-io/Factr-I) |
| Gateway logs | `~/.factr/logs/gateway.log` (or `factr logs`) |
| Sessions | `factr sessions browse` (reads state.db) |
