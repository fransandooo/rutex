# rutex — editor LaTeX nativo para Linux

Objetivo: tener lo cómodo de Overleaf (abres el proyecto, escribes, se compila solo
y ves el PDF al lado) en una app nativa, rápida, ligera y que no deje nada corriendo
al cerrarse. Pensada para Omarchy (Arch + Hyprland, Wayland), pero sirve en cualquier Linux.

## Decisiones técnicas

| Pieza | Elección | Por qué |
|---|---|---|
| Lenguaje | Rust | Rápido, binario único, sin GC |
| Interfaz | GTK4 + libadwaita (`gtk4`, `libadwaita`) | Nativo en Wayland, sigue el modo oscuro de Omarchy, ya instalado en el sistema, ~60–120 MB de RAM |
| Editor | GtkSourceView 5 (`sourceview5`) | Resaltado LaTeX, deshacer, búsqueda, números de línea y modo Vim ya resueltos y probados |
| Visor PDF | Poppler (`poppler-rs`) | Ya instalado; renderiza a Cairo directamente |
| Motor TeX | TeX Live nativo + `latexmk` (por defecto), Tectonic opcional | TeX Live es lo que usa Overleaf: máxima compatibilidad con plantillas de TFM, biber, pdfLaTeX/XeLaTeX/LuaLaTeX |
| Docker | **No** | Ver más abajo |

### Por qué no Docker

- Cada compilación pagaría el arranque del contenedor (~0,5–1 s) o habría que dejar un
  contenedor vivo en segundo plano, justo lo que queremos evitar.
- Obliga a tener el demonio de Docker activo (ahora mismo está desactivado en este equipo).
- Si la app se cierra mal, el contenedor puede quedar huérfano.
- En Arch, `pacman -S texlive-*` da TeX Live 2026 actualizado, sin capas extra.

Docker solo tendría sentido como backend opcional para reproducir exactamente la imagen de
Overleaf. Si algún día se añade, la app avisará de que hace falta Docker y limpiará sus
contenedores (etiquetados) al arrancar y al salir.

## Arquitectura

```
rutex (un solo proceso)
├── Hilo principal GTK
│   ├── Árbol de archivos del proyecto
│   ├── Editor (GtkSourceView) con autoguardado
│   ├── Visor PDF (solo páginas visibles, render en hilo aparte, caché acotada)
│   └── Panel de errores (parseo del .log → clic salta a la línea)
├── Compilador
│   ├── Se dispara al dejar de escribir (~800 ms) o con Ctrl+S
│   ├── latexmk -interaction=nonstopmode -synctex=1 -outdir=~/.cache/rutex/<proyecto>
│   ├── Si sigues escribiendo durante una compilación, se cancela y se relanza
│   └── Hijo en su propio grupo de procesos + PR_SET_PDEATHSIG
└── Vigilante de archivos (inotify) para cambios externos (git pull, otro editor)
```

### Ciclo de vida y recursos

- **Nada en segundo plano**: no hay servidor ni demonio. Al cerrar la última ventana el
  proceso termina.
- **Los procesos de compilación nunca quedan huérfanos**: cada `latexmk` (y los `pdflatex`/`biber`
  que lance) va en su propio grupo de procesos. Al cancelar o salir se mata el grupo entero, y
  `PR_SET_PDEATHSIG` hace que el kernel lo mate si la app se cuelga o crashea.
- **Instancia única** (GApplication por D-Bus): abrir rutex otra vez enfoca la ventana existente
  en vez de duplicar el proceso.
- **CPU en reposo ≈ 0**: todo funciona por eventos, sin sondeos.
- **Los auxiliares (.aux, .log, .synctex.gz…) van a `~/.cache/rutex/`**, no a la carpeta del
  proyecto: el proyecto queda limpio (como en Overleaf) y la siguiente compilación es incremental.
- **PDF**: solo se renderizan las páginas visibles (±1) al zoom actual, con un límite de
  memoria para la caché. Al recompilar se mantiene la posición de scroll.
- Compilación en modo release con LTO y `strip`.

### Integración con Omarchy

- Archivo `.desktop` + icono en `~/.local/share/applications` → aparece en el lanzador (Super+Espacio).
- Atajo opcional en `~/.config/hypr/bindings.conf`, con la convención de Omarchy:
  `bindd = SUPER SHIFT, L, LaTeX, exec, omarchy-launch-or-focus dev.fran.Rutex "uwsm-app -- rutex"`
- Sigue el modo oscuro del sistema (libadwaita). Más adelante: colores del tema de Omarchy en el editor.

## Hoja de ruta

1. **MVP**: abrir una carpeta de proyecto, árbol de archivos, editor con resaltado, autoguardado
   y autocompilado, visor PDF, panel de errores y ciclo de vida limpio de procesos.
2. **Navegación**: SyncTeX en los dos sentidos (clic en el PDF → línea, y al revés), marcas de
   error en el editor, pantalla de proyectos recientes y plantillas (artículo, TFM, apuntes).
3. **Productividad**: autocompletado de `\ref`/`\cite`/comandos (propio o `texlab` bajo demanda),
   esquema de secciones, corrector ortográfico (libspelling) y tema de Omarchy.
4. **Proyectos enormes**: preámbulo precompilado, `\includeonly` automático del capítulo que
   estás editando y modo borrador.

## Dependencias del sistema

```bash
# Ya instaladas: gtk4 libadwaita gtksourceview5 poppler-glib
sudo pacman -S --needed texlive-basic texlive-latex texlive-latexrecommended texlive-latexextra \
  texlive-fontsrecommended texlive-bibtexextra texlive-mathscience texlive-langspanish \
  texlive-binextra texlive-xetex texlive-luatex biber
# (texlive-binextra trae latexmk y synctex). Para tenerlo todo: sudo pacman -S texlive-meta
```
