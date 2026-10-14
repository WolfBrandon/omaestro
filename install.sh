#!/usr/bin/env bash
# Installs OMAESTRO as an app: builds it, copies it to ~/.local/bin, and adds its icon and an
# entry in the app launcher (Super+Space). Run it again after changes to update the app.
#
#   ./install.sh              install or update
#   ./install.sh --uninstall  remove the app, its icon and the launcher entry
#                             (settings in ~/.config/omaestro and the library index stay)
set -euo pipefail
cd "$(dirname "$0")"

BIN="$HOME/.local/bin/omaestro"
ICONS="$HOME/.local/share/icons/hicolor"
APPS="$HOME/.local/share/applications"
ENTRY="$APPS/org.omarchy.omaestro.desktop"
MARK="# Written by OMAESTRO's install.sh"

# Tell the launcher and icon theme about the changes.
refresh() {
  gtk-update-icon-cache -f -t "$ICONS" >/dev/null 2>&1 || true
  update-desktop-database "$APPS" >/dev/null 2>&1 || true
}

if [[ ${1:-} == --uninstall ]]; then
  rm -fv "$BIN" "$ICONS/scalable/apps/omaestro.svg" "$ICONS/256x256/apps/omaestro.png" "$ENTRY"
  refresh
  echo "OMAESTRO removed. Settings in ~/.config/omaestro and the library index are untouched."
  exit 0
fi

echo "Building…"
cargo build --release

# Copy, then rename into place, so an OMAESTRO that's already running isn't disturbed.
mkdir -p "$(dirname "$BIN")"
cp target/release/omaestro "$BIN.new"
mv -f "$BIN.new" "$BIN"

# The icon: the drawing itself, and a 256px picture for anything that wants one.
install -Dm644 assets/omaestro.svg "$ICONS/scalable/apps/omaestro.svg"
if command -v rsvg-convert >/dev/null; then
  mkdir -p "$ICONS/256x256/apps"
  rsvg-convert -w 256 -h 256 assets/omaestro.svg -o "$ICONS/256x256/apps/omaestro.png"
fi

# The launcher entry. "omaestro launch" brings back a running OMAESTRO, or opens a new one in
# the default terminal with the app-id org.omarchy.omaestro (for any window rules you add).
mkdir -p "$APPS"
if [[ -f $ENTRY ]] && ! grep -qF "$MARK" "$ENTRY"; then
  cp "$ENTRY" "$ENTRY.bak"
  echo "Kept your old launcher entry as $ENTRY.bak"
fi
cat >"$ENTRY" <<EOF
[Desktop Entry]
$MARK
Type=Application
Version=1.0
Name=OMAESTRO
GenericName=Music Player
Comment=Your music library, Spotify and YouTube Music, here or on Sonos
Exec=$BIN launch
Icon=omaestro
Terminal=false
Categories=AudioVideo;Audio;Player;
Keywords=music;player;sonos;spotify;youtube;library;
StartupNotify=true
StartupWMClass=org.omarchy.omaestro
EOF
refresh

echo "Installed $("$BIN" --version). Find it with Super+Space → OMAESTRO."
