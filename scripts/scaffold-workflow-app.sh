#!/usr/bin/env bash
# Clone LighFixture xcode template → new workflow fixture app.
# Usage: ./scripts/scaffold-workflow-app.sh LighFeed dev.ligh.Feed
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
NAME="${1:?app name e.g. LighFeed}"
BUNDLE="${2:?bundle id e.g. dev.ligh.Feed}"
DEST="$ROOT/fixtures/$NAME"
[[ -d "$DEST" ]] && { echo "exists: $DEST"; exit 1; }

cp -R "$ROOT/fixtures/LighFixture" "$DEST"
mv "$DEST/LighFixture" "$DEST/$NAME"
mv "$DEST/LighFixture.xcodeproj" "$DEST/$NAME.xcodeproj"

for f in "$DEST/$NAME.xcodeproj/project.pbxproj" \
         "$DEST/$NAME.xcodeproj/xcshareddata/xcschemes/$NAME.xcscheme" \
         "$DEST/$NAME/Info.plist" \
         "$DEST/$NAME/${NAME}App.swift" \
         "$DEST/$NAME/ContentView.swift"; do
  [[ -f "$f" ]] || continue
  sed -i '' -e "s/LighFixture/$NAME/g" -e "s/dev.ligh.Fixture/$BUNDLE/g" "$f"
done
# scheme file may still be named LighFixture.xcscheme before rename
if [[ -f "$DEST/$NAME.xcodeproj/xcshareddata/xcschemes/LighFixture.xcscheme" ]]; then
  mv "$DEST/$NAME.xcodeproj/xcshareddata/xcschemes/LighFixture.xcscheme" \
     "$DEST/$NAME.xcodeproj/xcshareddata/xcschemes/$NAME.xcscheme"
  sed -i '' -e "s/LighFixture/$NAME/g" "$DEST/$NAME.xcodeproj/xcshareddata/xcschemes/$NAME.xcscheme"
fi

echo "✓ scaffolded $DEST ($BUNDLE)"
