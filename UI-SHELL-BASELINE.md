# Factr-I shell baseline (S1)

The existing tiling tree remains the layout authority. The new regression suite records pane membership, tab order and split axes without coupling to generated node IDs:

| Preset | Current arrangement |
| --- | --- |
| Default | Sessions, Workspace, Terminal/Files/Changes as three columns |
| Basic | Same columns; all right tools rest |
| Focus | Sessions beside Workspace/Files/Changes, with a resting Terminal below |
| Terminal deck | Sessions, Workspace, Files/Changes above a Terminal row |
| Quad | Sessions/Files beside Workspace, above Terminal beside Changes |

`src/components/pane-shell/tree/ui-shell-baseline.test.ts` also records a user-dragged tree, persistence after drag, stable placement during late contributions, Bots as an enforced Sessions tab, conditional Routines registration and its remembered collapsed group, and late Bots Screen workspace tabs surviving every preset and staying closed after Close.

The Basic empty-right-zone assertion is deliberately `it.fails` in S1. Applying Default then Basic closes all three right-zone tools. The desired condition is visible Terminal content, with Terminal unhidden and its zone expanded; S2 must remove Terminal from Basic's resting tools and make this assertion pass normally. Done currently only exits editing, so preset application is the state-changing trigger reproduced here. This store regression does not claim a pixel-level reproduction of the packaged app.

Existing coverage to preserve and run (paths relative to `desktop/app`):

- `src/components/pane-shell/tree/mode-layout-memory.test.ts`: every Advanced preset against both Simple sidebar arrangements, pane widths/heights, hidden tabs, dismissals, custom tree, unfinished resize, independent persisted keys, migration and reload. Advanced keeps `factr.desktop.layoutTree.v2`; Simple uses the `.simple` scope. Other layout keys are listed in `src/lib/layout-persistence.ts`.
- `src/components/pane-shell/tree/dock-enforce.test.ts`: Bots boot enforcement, intra-session drag, next-boot repair, active Sessions reachability, Routines edge enforcement.
- `src/components/pane-shell/tree/resting-presets.test.ts`: resting stores, terminal rail, reapplication and active-tab preservation.
- `src/components/pane-shell/tree/tool-pane-toggle.test.ts`: terminal toggles in user stacks, workspace focus and close/reopen.
- `src/app/chat/session-drag.test.ts`: visible composer targeting, edge split, non-chat/sidebar refusal and single-surface handoff.
- `src/app/chat/new-session-drag.test.ts` and `src/app/chat/sidebar/sessions-section-new-session-drag.test.tsx`: new-session drag and sidebar gesture behavior.
- `src/components/pane-shell/tree/multi-tab-drag.test.ts`: multi-tab moves, order, no-op re-drops and source metadata.

Targeted command from `desktop/app` after dependencies are installed:

```sh
rtk npm run test:ui -- src/components/pane-shell/tree/ui-shell-baseline.test.ts src/components/pane-shell/tree/mode-layout-memory.test.ts src/components/pane-shell/tree/dock-enforce.test.ts src/components/pane-shell/tree/resting-presets.test.ts src/components/pane-shell/tree/tool-pane-toggle.test.ts src/app/chat/session-drag.test.ts src/app/chat/new-session-drag.test.ts src/app/chat/sidebar/sessions-section-new-session-drag.test.tsx src/components/pane-shell/tree/multi-tab-drag.test.ts
```

Validation results belong in CHECKPOINT.md. S1 adds no production changes or dependencies. Plugin tests here exercise the shell adoption contract with the actual contribution identities and dock flags; plugin lifecycle and packaged GUI checks remain separate.
