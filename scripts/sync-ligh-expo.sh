#!/usr/bin/env bash
# Sync packages/ligh-expo into a consumer Expo app (EAS-safe vendor copy).
# Usage: ./scripts/sync-ligh-expo.sh /path/to/ExpoApp
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/packages/ligh-expo"
DEST_APP="${1:?usage: sync-ligh-expo.sh /path/to/ExpoApp}"
DEST="$DEST_APP/packages/ligh-expo"

if [[ ! -f "$SRC/app.plugin.js" ]]; then
  echo "missing $SRC" >&2
  exit 1
fi
if [[ ! -d "$DEST_APP" ]]; then
  echo "app not found: $DEST_APP" >&2
  exit 1
fi

mkdir -p "$DEST"
rsync -a --delete \
  --exclude node_modules \
  --exclude .DS_Store \
  "$SRC/" "$DEST/"

# Ensure package.json dependency (file: vendor) if package.json exists
PKG_JSON="$DEST_APP/package.json"
if [[ -f "$PKG_JSON" ]]; then
  node -e '
    const fs = require("fs");
    const p = process.argv[1];
    const j = JSON.parse(fs.readFileSync(p, "utf8"));
    j.devDependencies = j.devDependencies || {};
    j.dependencies = j.dependencies || {};
    const ver = "file:./packages/ligh-expo";
    const name = "@mm-labs/ligh-expo";
    for (const old of ["@ligh/expo", "@mattisky999/ligh-expo", "@mm-labs/ligh-expo"]) {
      if (j.dependencies[old]) delete j.dependencies[old];
      if (j.devDependencies[old]) delete j.devDependencies[old];
    }
    j.devDependencies[name] = ver;
    fs.writeFileSync(p, JSON.stringify(j, null, 2) + "\n");
    console.log("package.json →", name, "=", ver);
  ' "$PKG_JSON"
fi

echo "synced → $DEST"
echo "Add plugin \"@mm-labs/ligh-expo\" to app.json plugins[] (first is fine)."
echo "Or: npm i -D @mm-labs/ligh-expo"
