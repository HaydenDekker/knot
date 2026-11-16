#!/usr/bin/env bash
# deploy-skills.sh — deploy the knot skill set to the host target
# (~/.agents), knot namespace ONLY. The host is a deployment target,
# not a repo (harness plan 012): deploy is a file act, never a git act.
#   usage: deploy-skills.sh [--committed] [--status]
#     --committed  deploy from git HEAD (default: working tree)
#     --status     dry run (itemized, no writes)
# Layout: the `knot` router -> ~/.agents/skills/ ; every other knot*
# sub-skill -> ~/.agents/skills-library/ . Prune is name-scoped to
# knot/knot-* — it can never touch another set's directories.
set -eu
KROOT=$(cd "$(dirname "$0")/.." && pwd)
TGT="$HOME/.agents"
MODE="" ; SRC=".agents/skills"
ARGS=("$@"); for a in "${ARGS[@]}"; do
  case "$a" in --committed) MODE=committed;; --status) ARGS_DRY=1;; esac
done
DRY=${ARGS_DRY:-0}
WORK=$SRC
if [ "$MODE" = committed ]; then
  WORK=$(mktemp -d); (cd "$KROOT" && git archive HEAD $SRC) | tar -x -C "$WORK"; WORK="$WORK/$SRC"
fi
opt="-a"; [ "$DRY" = 1 ] && opt="-ani"
mkdir -p "$TGT/skills" "$TGT/skills-library"
for d in "$KROOT/$WORK"/*/; do
  name=$(basename "$d")
  case "$name" in
    knot) rsync $opt "$d" "$TGT/skills/knot/";;
    knot-*) rsync $opt "$d" "$TGT/skills-library/$name/";;
    *) echo "skip (not knot namespace): $name";;
  esac
done
# prune knot-namespace dirs the source no longer ships (scoped glob only)
for d in "$TGT"/skills-library/knot-* "$TGT"/skills/knot; do
  [ -d "$d" ] || continue
  name=$(basename "$d")
  [ -d "$KROOT/$WORK/$name" ] || { echo "prune: $d"; [ "$DRY" = 0 ] && rm -rf "$d"; true; }
done
echo "deploy-skills: $([ "$DRY" = 1 ] && echo 'status (dry run)' || echo done)"
