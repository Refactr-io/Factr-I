# Optional Skills

Official skills maintained by Refactr that are **not activated by default**.

These skills ship with the factr-backend repository but are not copied to
`~/.factr/skills/` during setup. They are bundled with the app and can be enabled from the CLI:

```bash
factr skills browse               # browse all skills, official shown first
factr skills browse --source official  # browse only official optional skills
factr skills search <query>       # finds optional skills labeled "official"
factr skills install <identifier> # copies to ~/.factr/skills/ and activates
```

## Why optional?

Some skills are useful but not broadly needed by every user:

- **Niche integrations** — specific paid services, specialized tools
- **Experimental features** — promising but not yet proven
- **Heavyweight dependencies** — require significant setup (API keys, installs)

By keeping them optional, we keep the default skill set lean while still
providing curated, tested, official skills for users who want them.
