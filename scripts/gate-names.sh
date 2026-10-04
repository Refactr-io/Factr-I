#!/usr/bin/env bash
# Name gate: fails when any file, path or image in the tree carries a retired
# project name or old brand art.
#   scripts/gate-names.sh [ROOT]      (default: the repository root)
# Exempt: the regions of THIRD_PARTY_NOTICES.md fenced by <!-- verbatim:start -->
# and <!-- verbatim:end --> (upstream copyright/permission texts), and the
# third-party names in ALLOW below
# (npm packages, a font, model ids, math/English phrases). Letters of the
# retired names are hex-escaped so this file does not trip itself.
# Also prints every image file in the tree so the art can be eyeballed.
set -uo pipefail
ROOT="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
cd "$ROOT" || exit 2
command -v rg >/dev/null || { echo "gate: ripgrep (rg) is required" >&2; exit 2; }

W='(?:\x6acode|\x73overeign|\x68ermes|\x65vestack|\x6eous ?research|\x6eous|\x61kira)'
PAT="(?i:\\b${W}\\b|\x70rime[ _-]?agent|\\b\x70rime\\b)|(?i:\x6acode|\x73overeign|\x68ermes|\x65vestack|\x6eous[ _-]?research|\x61kira|1\x6aehuang|\x74eknium|\x63hronos|\x68ir0-pixel)|(?<![A-Za-z])(?:[Nn]ous(?![a-z])|\x4eOUS(?![A-Z])|[Pp]rime(?![a-z])|\x50RIME(?![A-Z]))"
# brand-art file names (in addition to PAT on every path)
ART='(?i)(caduceus|\x6eous-|\x68ermes(-frames|-logo|-icon|-mark|\.png))'

out="$(mktemp)"; trap 'rm -f "$out"' EXIT
{
  rg --hidden --pcre2 --text --no-heading --line-number --color never \
     -g '!.git' -g '!node_modules' -g '!target' -g '!target-*' -g '!.venv' -g '!dist' -g '!release' -g '!THIRD_PARTY_NOTICES.md' -e "$PAT" . || true
  # the notices file is scanned with its verbatim regions removed
  python3 -c '
import re, sys
t = open("THIRD_PARTY_NOTICES.md", encoding="utf-8").read().split("\n")
keep, on = [], True
for i, l in enumerate(t, 1):
    if "<!-- verbatim:start -->" in l: on = False; continue
    if "<!-- verbatim:end -->" in l: on = True; continue
    if on: print("./THIRD_PARTY_NOTICES.md:%d:%s" % (i, l))
' | rg --pcre2 -e "$PAT" || true
  rg --hidden --files -g '!.git' -g '!node_modules' -g '!target' -g '!target-*' -g '!.venv' -g '!dist' -g '!release' . | rg --pcre2 -e "$PAT" -e "$ART" | sed 's/^/NAME: /' || true
} | PAT="$PAT" python3 -c '
import os, re, sys
pat = re.compile(os.environ["PAT"])
allow = ["\x68ermes-parser", "\x68ermes-estree", "\x70rimeorder",
         "\x70rime numbers", "is \x70rime", "\x70rime mixing", "\x70rime awards",
         "\x70rime-editing", "web_search_\x70rime", "web-search-\x70rime",
         "wan-3.0-\x70rime", "Wan 3.0 \x50rime", "jackal_\x70rime_cert"]
for line in sys.stdin:
    body = line.split(":", 2)[-1] if not line.startswith("NAME: ") else line
    for a in allow:
        body = body.replace(a, "")
    if line.startswith("NAME: ") or pat.search(body):
        sys.stdout.write(line)
' >"$out"

echo "images in tree (eyeball for old brand art):"
rg --hidden --files -g '!.git' -g '!node_modules' -g '!target' -g '!target-*' -g '!.venv' -g '!dist' -g '!release' -g '*.{png,jpg,jpeg,gif,webp,svg,ico,icns,bmp,tiff}' . | LC_ALL=C sort | sed 's/^/  /'

# personal data: the maintainers' real home path / names, or a personal mail
# address. Fictional test homes and placeholder mailboxes (you@, user@...) are fine.
rg --hidden --pcre2 --text --no-heading --line-number --color never -g '!.git' -g '!node_modules' -g '!target' -g '!target-*' -g '!.venv' -g '!dist' -g '!release' \
   -e '(?i)\x72ameel(?!ement)|\x72imopixel|\x74okenspend|\x61dvancelabs|\x6aeremyh' \
   -e '(?i)(?<![A-Za-z0-9._%+-])(?!(?:you|user|user\+tag|agent|your|me|name|test|factr)@)[A-Za-z0-9._%+-]+@(?:[A-Za-z0-9-]+\.)*(?:gmail|googlemail|outlook|hotmail|icloud|yahoo|proton|protonmail|live)\.[a-z.]+' \
   . | sed 's/^/PERSONAL: /' >>"$out" || true
# no file over 10 MB in the tree
git ls-files -z | xargs -0 find -type f -size +10M 2>/dev/null | sed 's/^/LARGE: /' >>"$out"

# the Factr-I mark and app icons must be present
for f in docs/assets/factr-mark.png desktop/app/public/factr-mark.png desktop/app/assets/icon.png \
         desktop/app/assets/icon.icns desktop/app/assets/icon.ico desktop/app/public/apple-touch-icon.png \
         desktop/app/public/favicon-16.png desktop/app/public/favicon-32.png; do
  [ -s "$f" ] || echo "MISSING: $f" >>"$out"
done

n=$(wc -l <"$out" | tr -d ' ')
if [ "$n" -ne 0 ]; then
  head -n "${GATE_MAX:-200}" "$out" | cut -c1-240
  [ "$n" -gt 200 ] && echo "... ($n lines total)"
  echo "gate: FAIL ($n lines with retired names)"
  exit 1
fi
echo "gate: OK (0 hits)"
