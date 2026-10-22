#!/usr/bin/env bash
# Builds the dashboard UI that the desktop executable embeds.
#
#   scripts/build-desktop.sh
#
# Installs the locked dependencies, builds desktop-ui/ with an empty VITE_API_BASE (the bundle then
# calls the API with relative /api/... URLs served by the window's own origin) and checks the result.
# The embedding step reads desktop-ui/dist at cargo build time; dist/ is generated and not committed.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ui_dir="$root/desktop-ui"

cd "$ui_dir"
npm ci
# Set but empty on purpose: Vite then bakes an empty API base (same origin) into the bundle.
VITE_API_BASE= npm run build

if grep -rqF "localhost:8090" dist; then
	echo "error: the generated UI still points at an absolute API base (localhost:8090)" >&2
	exit 1
fi
if [ ! -f dist/index.html ]; then
	echo "error: this build embeds no UI (index.html not found)" >&2
	exit 1
fi
echo "built $ui_dir/dist"
