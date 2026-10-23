# rutex

Editor LaTeX nativo para Linux, escrito en Rust. Ofrece lo cómodo de Overleaf (abres el
proyecto, escribes, se compila solo y ves el PDF al lado) sin pagar, sin límite de
proyectos, sin Docker y sin dejar procesos en segundo plano.

Pensado para [Omarchy](https://omarchy.org) (Arch + Hyprland), pero funciona en cualquier
escritorio Linux con GTK4.

## Qué hace

- **Compilación automática**: al dejar de escribir (~0,8 s) guarda y compila con `latexmk`.
  Si escribes mientras compila, encola otra compilación en vez de interrumpirla.
- **Visor PDF integrado**: renderiza solo las páginas visibles en un hilo aparte. Al
  recompilar no parpadea y conserva el scroll.
- **SyncTeX**: doble clic en el PDF salta a la línea del código, y `Ctrl+J` salta del código
  al PDF.
- **Panel de errores y avisos**: haz clic en uno para ir al archivo y la línea.
- **Proyectos con varios archivos**: árbol de archivos, `\input`/`\include`, detección del
  archivo principal y comentarios mágicos `% !TEX root` y `% !TEX program`.
- **Motor automático**: pdfLaTeX por defecto y XeLaTeX si usas `fontspec`, `unicode-math` o
  `polyglossia`. Respeta tu `latexmkrc`.
- **Carpeta limpia**: los `.aux`, `.log`, etc. van a `~/.cache/rutex/`, no a tu proyecto.
- **Modo Vim** opcional (menú → Modo Vim).
- **Tema claro/oscuro** según el sistema.

## Instalación

```bash
# 1. TeX Live (una vez)
sudo pacman -S --needed texlive-basic texlive-latex texlive-latexrecommended \
  texlive-latexextra texlive-fontsrecommended texlive-bibtexextra texlive-mathscience \
  texlive-langspanish texlive-binextra texlive-xetex texlive-luatex biber

# 2. rutex (en ~/.local, sin sudo)
./install.sh
```

Aparece en el lanzador (Super+Espacio) como **rutex**. También puedes abrirlo desde la
terminal: `rutex ~/tfm` o `rutex capitulo.tex`.

Si quieres un atajo en Hyprland, añádelo a `~/.config/hypr/bindings.conf`:

```
bindd = SUPER SHIFT, L, LaTeX, exec, omarchy-launch-or-focus dev.fran.Rutex "uwsm-app -- rutex"
```

Para desinstalarlo: `./install.sh --uninstall`.

## Atajos

| Atajo | Acción |
|---|---|
| `Ctrl+S` / `Ctrl+Intro` | Guardar y compilar ya |
| `Ctrl+J` | Ir del código al PDF |
| Doble clic en el PDF | Ir del PDF al código |
| `Ctrl+F` | Buscar |
| `Ctrl+E` | Panel de problemas |
| `Ctrl++` / `Ctrl+-` / `Ctrl+0` / `Ctrl+rueda` | Zoom del PDF |
| `Ctrl+N` | Nuevo archivo |
| `Ctrl+O` | Abrir proyecto |
| `F9` | Mostrar u ocultar archivos |
| `Ctrl+W` / `Ctrl+Q` | Cerrar ventana / salir |

## Recursos y ciclo de vida

- Es un solo proceso y una sola instancia: abrirlo otra vez enfoca la ventana existente.
- Cuando no haces nada no usa CPU, porque todo funciona por eventos.
- Al cerrar la última ventana el proceso termina. Cada compilación va en su propio grupo de
  procesos con `PR_SET_PDEATHSIG`, así que ni `latexmk` ni `pdflatex` ni `biber` se quedan
  vivos, aunque rutex se cierre de golpe.
- La caché de páginas del PDF tiene un límite de 128 MB.

Más detalles en [docs/DISENO.md](docs/DISENO.md).

## Desarrollo

```bash
cargo run -- ruta/al/proyecto
cargo test
```

Depende de `gtk4`, `libadwaita`, `gtksourceview5` y `poppler-glib` (en Omarchy ya vienen
instalados).

## Licencia

GPL-3.0-or-later (usa Poppler, que es GPL).
