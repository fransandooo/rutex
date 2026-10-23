#!/usr/bin/env bash
# Instala rutex en ~/.local (sin sudo). Uso: ./install.sh [--uninstall]
set -euo pipefail
cd "$(dirname "$0")"

APP_ID=dev.fran.Rutex
BIN="$HOME/.local/bin/rutex"
DESKTOP="$HOME/.local/share/applications/$APP_ID.desktop"
ICON="$HOME/.local/share/icons/hicolor/scalable/apps/$APP_ID.svg"

if [[ "${1:-}" == "--uninstall" ]]; then
  rm -f "$BIN" "$DESKTOP" "$ICON"
  update-desktop-database "$HOME/.local/share/applications" 2>/dev/null || true
  echo "rutex desinstalado. (La caché de compilación está en ~/.cache/rutex y la configuración en ~/.config/rutex)"
  exit 0
fi

cargo build --release
install -Dm755 target/release/rutex "$BIN"
install -Dm644 data/$APP_ID.svg "$ICON"
install -Dm644 data/$APP_ID.desktop "$DESKTOP"
# Ruta absoluta: el lanzador no siempre hereda el PATH de la shell.
sed -i "s|^Exec=rutex|Exec=$BIN|" "$DESKTOP"
update-desktop-database "$HOME/.local/share/applications" 2>/dev/null || true
gtk-update-icon-cache -q -t "$HOME/.local/share/icons/hicolor" 2>/dev/null || true

echo "✓ rutex instalado en $BIN"
if ! command -v latexmk >/dev/null; then
  cat <<'MSG'

⚠ Falta TeX Live. Instálalo con:
  sudo pacman -S --needed texlive-basic texlive-latex texlive-latexrecommended \
    texlive-latexextra texlive-fontsrecommended texlive-bibtexextra texlive-mathscience \
    texlive-langspanish texlive-binextra texlive-xetex texlive-luatex biber
MSG
fi
