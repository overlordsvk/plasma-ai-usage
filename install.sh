#!/bin/sh
# Build the fetcher, put it in ~/.local/bin and (re)install the Plasma widget.
set -e
cd "$(dirname "$0")"
cargo build --release
install -Dm755 target/release/ai-usage "$HOME/.local/bin/ai-usage"
kpackagetool6 -t Plasma/Applet -u plasmoid 2>/dev/null || kpackagetool6 -t Plasma/Applet -i plasmoid
echo "Done. Add 'AI Usage' to the panel (right-click panel > Add Widgets)."
echo "After updating the QML, restart plasmashell: systemctl --user restart plasma-plasmashell"
