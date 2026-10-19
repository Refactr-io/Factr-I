# UI-SHELL checkpoint

## Scope and rules
- Branch `feat/ui-shell`, base `a834c8e1`. Only this worktree and isolated `/private/tmp/factri-ui-shell` test data; no credentials, release checkout, benchmark output, remotes or pushes.
- Read COMMON-RULES.md, all ADVICE-FROM-PARENT.md (including UI amendments), ui-shell-handoff.md, REQUIREMENTS.md, claude-code-hands-on.md, OPUS-PLAN.md, SOL-ADVICE.md, FABLE-ADVICE.md, DESIGN-refactr.md and RTK.md.
- S1 baseline regressions; S2 commands on existing tree + Basic fix; S3 sizes/overlays/Fill; S4 focused-chat header + Session; S5 pane chrome; S6 editor removal last. Settings gear and internal presets/tree/zones APIs survive. Fonts, activity, HUD and profiles belong to other worktrees.
- Each step gets a before and validated commit, with required co-author trailer.

## Evidence
- Verify tier: graph `factri-int10-app`, generation `2026-10-19T21:05:07Z`, ready, fast index, source is sibling integration checkout. Compare touched local source before edits. Coverage has no recorded gaps for store/presets/layout-persistence/layout-presets/session-drag/plugin; tests excluded by design, read directly. Graph coverage is best effort.
- `applyLayoutPreset` inbound: onboarding assembly, setup/editor, pane-focus bridge; outbound applyTree/asLayoutIntent. Preserve internal callers.
- Initial `git status --short`: clean. `df -h /`: 41 GiB initially, 39 GiB after reading.

## S1 before
- No code changed yet. Existing tests already cover mode memory, tool toggles and plugin docking; extend these with preset/current-layout baselines and expected Basic regression.
- Validation pending. No node_modules exists in this worktree.

## Advice
- CLI advice unavailable by instruction; never invoke claude. Parent amendments are authoritative: no canonical-tree rewrite, preserve tree and plugin placement, snapshot old persisted state before migration, Fill covers workspace, minimum stack height, focused chat owns header, keep Settings gear.
- Install coordination: found three existing npm ci processes (91039, 91663, 91932). Do not start another until installs finish; continue source/test work meanwhile.

## Next
- Capture S1 regression cases and baseline notes. Install dependencies only when no other install is active, then execute S1 checks.
