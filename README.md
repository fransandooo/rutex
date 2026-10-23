# rutex

A fast, native LaTeX editor for Linux, written in Rust.

rutex gives you the comfortable parts of Overleaf (open a project, type, and the PDF updates
next to your code) without a subscription, without project limits, without Docker and
without leaving anything running in the background.

It is built with GTK4 + libadwaita and made with [Omarchy](https://omarchy.org)
(Arch + Hyprland) in mind, but it runs on any modern Linux desktop.

> **Note:** the interface is currently in Spanish. An English translation is planned.

## Features

- **Live compilation**: when you stop typing (~0.8 s), rutex saves and compiles with
  `latexmk`. If you keep typing during a build, another build is queued instead of killing
  the running one, so the `.aux` files are never left corrupted.
- **Built-in PDF viewer**: only the visible pages are rendered, on a separate thread. The
  viewer doesn't flicker and keeps your scroll position when it reloads.
- **SyncTeX**: double-click the PDF to jump to the source line, and press `Ctrl+J` to jump from
  the source to the PDF.
- **Errors and warnings panel**: click an entry to open that file at that line.
- **Multi-file projects**: file tree, `\input`/`\include`, automatic main-file detection, and
  the `% !TEX root` and `% !TEX program` magic comments.
- **Automatic engine choice**: pdfLaTeX by default, and XeLaTeX when the document uses
  `fontspec`, `unicode-math` or `polyglossia`. Your `latexmkrc` is respected.
- **Clean project folders**: `.aux`, `.log` and other build files go to `~/.cache/rutex/`, not
  next to your sources.
- **Optional Vim mode** (menu → *Modo Vim*).
- **Light and dark themes** that follow the system.

## Requirements

| Component | Minimum version |
|---|---|
| GTK | 4.14 |
| libadwaita | 1.5 |
| GtkSourceView | 5.4 |
| Poppler (glib) | 0.82 |
| TeX distribution | TeX Live with `latexmk` and `synctex` |
| Rust (to build) | 1.85 (edition 2024), via [rustup](https://rustup.rs) |

### Arch Linux / Omarchy

```bash
sudo pacman -S --needed gtk4 libadwaita gtksourceview5 poppler-glib \
  texlive-basic texlive-latex texlive-latexrecommended texlive-latexextra \
  texlive-fontsrecommended texlive-bibtexextra texlive-mathscience texlive-binextra \
  texlive-xetex texlive-luatex biber
# Optional, for Spanish documents: texlive-langspanish
```

### Fedora

```bash
sudo dnf install gtk4-devel libadwaita-devel gtksourceview5-devel poppler-glib-devel \
  texlive-scheme-medium latexmk biber
```

### Debian / Ubuntu (24.04 or newer)

```bash
sudo apt install build-essential libgtk-4-dev libadwaita-1-dev libgtksourceview-5-dev \
  libpoppler-glib-dev texlive-latex-extra texlive-fonts-recommended texlive-xetex \
  texlive-luatex texlive-bibtex-extra latexmk biber
```

## Install

```bash
git clone https://github.com/fransandooo/rutex.git
cd rutex
./install.sh
```

This builds rutex in release mode and installs it into `~/.local`, so `sudo` isn't needed:

- the binary goes to `~/.local/bin/rutex`;
- a desktop entry and icon are added, so rutex shows up in your app launcher (on Omarchy,
  `Super+Space`).

To update, run `git pull && ./install.sh`. To remove rutex, run `./install.sh --uninstall`.

### Usage

Open it from the launcher, or from a terminal:

```bash
rutex ~/thesis          # open a project folder
rutex chapter.tex       # open a file (its project is detected)
```

The welcome screen lets you open a folder or start a new project. A new project gets a
`main.tex` template.

### Hyprland keybinding (optional)

Add this to `~/.config/hypr/bindings.conf`:

```
bindd = SUPER SHIFT, L, LaTeX, exec, omarchy-launch-or-focus dev.fran.Rutex "uwsm-app -- rutex"
```

## Keyboard shortcuts

| Shortcut | Action |
|---|---|
| `Ctrl+S` / `Ctrl+Enter` | Save and compile now |
| `Ctrl+J` | Jump from source to PDF |
| Double-click on PDF | Jump from PDF to source |
| `Ctrl+F` | Find |
| `Ctrl+E` | Toggle the problems panel |
| `Ctrl++` / `Ctrl+-` / `Ctrl+0` / `Ctrl+wheel` | PDF zoom |
| `Ctrl+N` | New file |
| `Ctrl+O` | Open project |
| `F9` | Toggle the file sidebar |
| `Ctrl+W` / `Ctrl+Q` | Close window / quit |

## Resource usage and lifecycle

- rutex is a single process with a single instance: launching it again focuses the existing
  window.
- It uses no CPU when idle, because everything is event-driven and nothing polls.
- Closing the last window ends the process.
- Every build runs in its own process group with `PR_SET_PDEATHSIG`. Closing a window kills
  its build, and if rutex crashes the kernel kills `latexmk` with it. At worst, a `pdflatex`
  or `biber` run that was already in progress finishes on its own.
- The cache of rendered PDF pages is capped at 128 MB.

More details (in Spanish) are in [docs/DISENO.md](docs/DISENO.md).

## Development

```bash
cargo run -- path/to/project
cargo test
```

## License

GPL-3.0-or-later. rutex links against Poppler, which is GPL-licensed.
