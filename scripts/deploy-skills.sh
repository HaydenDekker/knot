#!/usr/bin/env bash
# deploy-skills.sh — deploy the knot skill set to the host target
# (~/.agents), knot namespace ONLY. The host is a deployment target,
# not a repo (harness plan 012): deploy is a file act, never a git act.
#
#   usage: scripts/deploy-skills.sh [--committed] [--status]
#     --committed  deploy from git HEAD (default: working tree)
#     --status     dry run (itemized, no writes)
#
# Layout: the `knot` router -> ~/.agents/skills/ ; every other knot*
# sub-skill -> ~/.agents/skills-library/ . Prune is name-scoped to
# knot/knot-* — it can never touch another set's directories.
set -eu
KROOT=$(cd "$(dirname "$0")/.." && pwd)
TGT="$HOME/.agents"
SRC_ROOT="$KROOT/.agents/skills"
DRY=0
for a in "$@"; do
  case "$a" in
    --committed) TMP=$(mktemp -d); trap 'rm -rf "$TMP"' EXIT
                 (cd "$KROOT" && git archive HEAD .agents/skills) | tar -x -C "$TMP"
                 SRC_ROOT="$TMP/.agents/skills";;
    --status)    DRY=1;;
  esac
done
[ -d "$SRC_ROOT" ] || { echo "deploy-skills: no source set at $SRC_ROOT"; exit 1; }
opt="-a"; [ "$DRY" = 1 ] && opt="-ani"
mkdir -p "$TGT/skills" "$TGT/skills-library"

for d in "$SRC_ROOT"/*/; do
  name=$(basename "$d")
  case "$name" in
    knot)   rsync $opt "$d" "$TGT/skills/knot/";;
    knot-*) rsync $opt "$d" "$TGT/skills-library/$name/";;
    *)      echo "skip (not knot namespace): $name";;
  esac
done

# prune knot-namespace dirs the source no longer ships (scoped globs only)
for d in "$TGT"/skills-library/knot-* "$TGT"/skills/knot; do
  [ -d "$d" ] || continue
  name=$(basename "$d")
  if [ ! -d "$SRC_ROOT/$name" ]; then
    echo "prune: $d"
    [ "$DRY" = 0 ] && rm -rf "$d"
  fi
done
true
