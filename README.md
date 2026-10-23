# rutex

A fast, native LaTeX editor for Linux, written in Rust.

rutex gives you the comfortable parts of Overleaf (open a project, type, and the PDF updates
next to your code) without a subscription, without project limits, without Docker and
without leaving anything running in the background.

It is built with GTK4 + libadwaita and made with [Omarchy](https://omarchy.org)
(Arch + Hyprland) in mind, but it runs on any modern Linux desktop.

> **Note:** the interface is currently in Spanish. An English translation is planned.

## Features

**Writing**
- **Live compilation**: when you stop typing, rutex saves and compiles with `latexmk`. If you
  keep typing during a build, another build is queued instead of killing the running one, so
  `.aux` files are never left corrupted.
- **Tabs and a file tree**: real folders, plus a right-click menu to create files and
  folders, rename or move, set the main file, copy the path, show it in the file manager or
  move it to the trash. Images and PDFs open with the system's default app.
- **Autocompletion**: commands with tab stops; `\begin{` inserts whole environments (figure,
  table, align…); `\ref{` completes your labels and `\cite{` your `.bib` keys;
  `\includegraphics{` completes images and `\input{` completes chapters.
- **Editing shortcuts**: comment lines, bold and italic, duplicate and move lines,
  auto-closing `{ [ ( $`, and `\end{…}` inserted when you press Enter after `\begin{…}`.
- **Outline**: the chapters and sections of the whole document (following `\input`), one
  click away.
- **Quick open** (`Ctrl+P`) with fuzzy search over files, or sections with `@`.
- **Errors where they happen**: lines with errors and warnings are marked in the editor, and
  the problems panel opens the file at the right line.
- **Word count** of the whole document in the status bar.
- **Optional Vim mode**.

**Looks**
- **Follows your Omarchy theme live**: change it with `omarchy-theme-set` and the interface
  and the editor's syntax colors update immediately. On other desktops it follows the
  system's light/dark setting.
- Optional **dark mode for the PDF**, which repaints pages with your theme colors.

**PDF**
- **Built-in viewer** that renders only the visible pages, on a separate thread. It doesn't
  flicker and keeps your scroll position when it reloads.
- **SyncTeX**: double-click the PDF to jump to the source line, and press `Ctrl+J` to jump from
  the source to the PDF.

**Projects**
- **Templates**: article, TFM/thesis (chapters, biblatex + biber), lecture notes, Beamer
  slides and empty.
- **Automatic engine choice**: pdfLaTeX by default, XeLaTeX for `fontspec` / `unicode-math` /
  `polyglossia`. The `% !TEX program` and `% !TEX root` magic comments and your `latexmkrc`
  are respected.
- **Clean project folders**: build files go to `~/.cache/rutex/`.

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

Press `Ctrl+?` inside the app to see them all.

| Shortcut | Action |
|---|---|
| `Ctrl+S` / `Ctrl+Enter` | Save and compile now |
| `Ctrl+P` | Quick open (type `@` for sections) |
| `Ctrl+J` / double-click on PDF | Jump source → PDF / PDF → source |
| `Ctrl+Z` / `Ctrl+Y` / `Ctrl+Shift+Z` | Undo / redo |
| `Ctrl+F` / `Ctrl+G` | Find / go to line |
| `Ctrl+/` | Comment or uncomment lines |
| `Ctrl+B` / `Ctrl+I` | `\textbf{}` / `\textit{}` |
| `Ctrl+D` / `Ctrl+Shift+K` / `Alt+↑↓` | Duplicate / delete / move lines |
| `Ctrl+Space` | Show suggestions |
| `Ctrl+N` / `Ctrl+Shift+N` | New file / new project |
| `Ctrl+Shift+E` / `Ctrl+Shift+O` / `F9` | Files / outline / toggle sidebar |
| `Ctrl+E` | Problems panel |
| `Ctrl++` / `Ctrl+-` / `Ctrl+0` / `Ctrl+wheel` | PDF zoom |
| `Ctrl+,` | Preferences |
| `Ctrl+W` / `Ctrl+Shift+W` / `Ctrl+Q` | Close tab / close window / quit |

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
cargo test                 # unit tests
cargo test -- --ignored    # also compiles every template (needs TeX Live)
```

## License

GPL-3.0-or-later. rutex links against Poppler, which is GPL-licensed.
