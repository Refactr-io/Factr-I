# UI fonts checkpoint

## Scope and baseline
- Worktree: `/Users/rameelmalik/factri-ui-font-wt`, branch `feat/ui-fonts`.
- Read COMMON-RULES.md, ADVICE-FROM-PARENT.md, ui-fonts-handoff.md and its reference documents completely. Follow the specific font task over the batch boilerplate saying not to change families.
- `rtk git status --short --branch`: clean. `rtk proxy df -h /`: 42 GiB available.
- Before commit: `5c9eb99` (empty baseline commit, required trailer).
- Verify tier: graph `factri-int10-app`, ready, generation `2026-10-19T21:05:07Z`. Graph points to integration worktree, so all edited source is read locally. Terminal resolver trace has two direct callers (controller and setting). Coverage: styles.css partial (2,16,17,60,1833); tests/i18n/scripts excluded; broad scopes have pagination, not used to claim exhaustive graph coverage. Literal scans and direct source reads cover font configuration and test files.

## Inventory
- `src/styles.css`: seven Arimo variable subsets; JetBrains Mono regular/bold/italic; CSS/Tailwind font aliases; UI 13px, conversation 14/20, captions 12/16; root 16px.
- `src/themes/presets.ts` / `context.tsx`: duplicates CSS family defaults and applies them to each window; preserves profile chat-font override.
- `src/app/right-sidebar/terminal/{terminal-font,use-agent-terminal,use-terminal-session}.ts`: separate bundled family, 11px default, warm-up and live user font override. Preserve Nerd Font choices and PTYs.
- CodeMirror `components/chat/code-editor.tsx`; source/diff/Shiki/markdown and read-only terminal components use font-mono, several 11/12px code sizes. No Monaco dependency found in manifest.
- Shared UI button/input/dialog, settings primitives/navigation, sidebar row primitives, chat markdown, HUD (inherits CSS), quick entry/secondary windows (same renderer).
- Intro scene/viewport canvas and starmap canvas hardcode JetBrains Mono; image-generation placeholder hardcodes system mono. Keep the unrelated Collapse display branding.
- Electron `renderer-load-error-page.ts` intentionally self-contained system sans/mono when renderer assets fail. Boot UI uses renderer CSS. Embedded remote-page annotation and copied HTML deliberately use portable system fonts (no app font assets in guest documents).
- Font assets `src/fonts/`; package build copies root THIRD_PARTY_NOTICES.md. Self-host vendored OFL files without adding runtime dependencies.

## Advice
- CLI advice unavailable per parent. Continue safely; questions recorded in QUESTIONS-ui-fonts.md when relevant.
- Font subset decision: use official Latin-capable trimmed variable font, explicit platform fallbacks for missing scripts. Keep existing user font preferences.

## Next
1. Vendor fonts/licenses, replace old faces, preload primary face, validate assets, commit.
2. Centralize role scale and apply to code/terminals/chrome, targeted tests, commit.
3. Typecheck, touched-file ESLint, full UI Vitest, renderer build and isolated visual QA; record limitations, report, final commit.

## Milestone 1: self-hosted fonts
- Added official unmodified Pretendard Std Variable 1.3.9 and Fontsource IBM Plex Mono 5.3.0 (400/500/600, normal/italic, Latin + Latin Extended), licenses and packaged notices.
- Replaced Arimo and bundled JetBrains consumers in CSS/theme/terminal/intro/canvas; removed retired assets. Kept installed Nerd Font suggestions. Updated existing translated terminal-help strings only to correct the family name; no new keys.
- Preloaded primary UI and mono 400. Primary faces block briefly, secondary mono weights/styles swap.
- Payload: 574,108 -> 472,952 bytes, -101,156 bytes (17.6%). Independent WOFF2 magic/size check passed (initial ad-hoc regex check had an escaped-pattern bug, corrected to direct file checks; production test passed).
- Dependencies: one isolated `npm ci --no-audit --no-fund` in desktop, 1,191 packages, success; no runtime dependencies added. Fresh HOME/FACTR_HOME/FACTR_CONFIG_HOME/cache under /private/tmp/factri-ui-font-qa. Disk now 38 GiB.
- Validation: `cd desktop/app; ../node_modules/.bin/vitest run --project ui src/themes/font-assets.test.ts src/themes/presets.test.ts src/themes/context.test.tsx src/app/right-sidebar/terminal/terminal-font.test.ts --maxWorkers=2`: 4 files / 18 tests passed. Test covers all CSS assets, preloads, retired family references and font budget/licenses. `git diff --check` fixed license trailing whitespace in packaged notice copy.
- Re-read parent advice: new shell/activity amendments do not change this font task. No STOP.
- Next: role sizes/weights/tracking and terminal geometry, then targeted validation.
