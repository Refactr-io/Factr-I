# Bundled fonts

No runtime font requests leave the app.

- Pretendard Std Variable 1.3.9, official unmodified Latin build (100-900): https://github.com/orioncactus/pretendard/tree/v1.3.9/packages/pretendard-std/dist/web/variable/woff2
- IBM Plex Mono, @fontsource/ibm-plex-mono 5.3.0: Latin and Latin Extended WOFF2, 400/500/600 normal and italic. https://www.npmjs.com/package/@fontsource/ibm-plex-mono/v/5.3.0
- Both use SIL OFL 1.1. Original license texts are beside the assets and reproduced in the packaged root THIRD_PARTY_NOTICES.md.
- Payload: 472,952 bytes (Pretendard 291,680; IBM Plex Mono 181,272). Previous Arimo + JetBrains Mono: 574,108 bytes. Reduction: 101,156 bytes (17.6%).
- Pretendard Std omits CJK/Arabic; the platform fallback stack supplies missing scripts. Do not remove the system/emoji fallbacks.
- The primary variable face and mono 400 Latin are preloaded; primary faces use block, secondary mono faces use swap. Font loading never gates renderer mounting.
