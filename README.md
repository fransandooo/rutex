# rutex

A fast, native LaTeX editor for Linux, written in Rust.

rutex gives you the comfortable parts of Overleaf (open a project, type, and the PDF updates
next to your code) without a subscription, without project limits, without Docker and
without leaving anything running in the background.

It is built with GTK4 + libadwaita and made with [Omarchy](https://omarchy.org)
(Arch + Hyprland) in mind, but it runs on any modern Linux desktop.

The interface is in English by default, and Spanish is available in Preferences.

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
- **Errors where they happen**: lines with errors and warnings are marked in the editor.
  Hover over one to read the message, or click its margin icon to open the problems list.
- **Spell checking** as you type, in the document's language (from `babel`/`polyglossia`), that
  skips commands, math, labels, citation keys and file names.
- **Hover a `\ref` or `\cite`** to see what it points to (the caption, section title or entry),
  and **Ctrl+click** it to go to the `\label` or the `.bib` entry.
- **Find and replace in the whole project** (`Ctrl+Shift+F`).
- **Formula preview on hover**, rendered with your own preamble and macros.
- **Drag an image** onto the editor to get a complete `figure` (the file is copied into the
  project), and **paste cells from a spreadsheet** to get a `table`; or use Insert → Table from
  CSV.
- **References by DOI or arXiv id**: paste it, and the entry is added to your `.bib` and cited.
  **Zotero** (with Better BibTeX) works too.
- **Errors explained in plain words**: hovering an error shows what it means and how to fix it,
  for the ~40 most common LaTeX errors and warnings, offline. Optionally, with your own Anthropic
  API key (Preferences → AI help), an "Ask Claude" button in the problems list explains any error
  using the lines around it. Off by default; nothing is sent unless you press the button.
- **Export to Word** (☰ → Export to Word): for supervisors who review in Word. Equations become real Word
  equations, references keep their numbers, and TikZ drawings, plots, Gantt charts and pseudocode
  are inserted as images. Needs `pandoc` (`sudo pacman -S pandoc-cli`).
- **Symbol palette** (`Ctrl+Shift+M`) with search, and quick snippets: inside a formula `@a` →
  `\alpha`, `@8` → `\infty`, `@/` → a fraction; `BEQ` + `Tab` → an equation (also `BAL`, `BIT`,
  `BEN`, `BFI`, `BTA`, `BCAS`…).
- **Words per chapter** and an optional word goal, from the status bar.
- **Optional Vim mode**.

**Looks**
- **Follows your Omarchy theme live**: change it with `omarchy-theme-set` and the interface
  and the editor's syntax colors update immediately. On other desktops it follows the
  system's light/dark setting.
- **Dark mode for the PDF** (the moon button in the viewer), which repaints pages with your
  theme colors.
- Its own flat look: custom tabs, and a command center in the header that opens quick open.

**PDF**
- **Built-in viewer** that renders only the visible pages, on a separate thread. It doesn't
  flicker and keeps your scroll position when it reloads.
- **SyncTeX**: double-click (or `Ctrl+click`) the PDF to jump to the source line, and press `Ctrl+J`
  to jump from the source to the PDF.
- **Clickable links**: the table of contents, `\ref`, `\cite` and URLs work like in any PDF
  viewer, with a back button to return.
- **Fullscreen reading**: `F11` or the button in the viewer shows only the PDF; `Esc` goes back.

**Safety net**
- **Local version history** (`Ctrl+Shift+H`): automatic versions while you work, named versions
  (`Ctrl+Shift+S`), a readable diff against your current files and one-click restore. It is
  a private git repository in `~/.local/share/rutex/history/`; nothing is written into your
  project and nothing leaves your machine.
- **Check before submitting** (`Ctrl+Shift+R`): undefined references and citations, labels
  never referenced, uncited bibliography entries, duplicate labels, text sticking into the
  margin (worst first) and to-do notes, each one clickable.
- **Missing packages**: when a build fails because a `.sty` is missing, rutex tells you which
  package provides it (from TeX Live's own database, offline) and copies the `pacman` command.
- **No compile timeout**: Overleaf's free plan stops compiling after 10 seconds; a thesis with
  plots and a bibliography easily needs more.

**Projects**
- **A showcase demo**: "Try the demo" on the welcome screen opens a 16-page project with TikZ,
  PGFPlots, circuits, trees, tables, algorithms, code listings, a Gantt chart and a
  bibliography. Its source is in [`demo/`](demo/).
- **Templates**: article, thesis (chapters, biblatex + biber), lecture notes, Beamer slides
  and empty. They follow the interface language, including `babel` and folder names.
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
  texlive-xetex texlive-luatex texlive-pictures biber
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
| `Ctrl+J` / `Ctrl+click` or double-click on PDF | Jump source → PDF / PDF → source |
| Click on a PDF link | Follow it (contents, references, citations, URLs); the back button returns |
| `Ctrl+Z` / `Ctrl+Y` / `Ctrl+Shift+Z` | Undo / redo |
| `Ctrl+F` / `Ctrl+Shift+F` / `Ctrl+G` | Find / find in project / go to line |
| `Ctrl+Shift+H` / `Ctrl+Shift+S` | Version history / save a named version |
| `Ctrl+Shift+R` | Check before submitting |
| `Ctrl+click` on a `\ref` or `\cite` key | Go to its `\label` or `.bib` entry |
| `Ctrl+/` | Comment or uncomment lines |
| `Ctrl+B` / `Ctrl+I` | `\textbf{}` / `\textit{}` |
| `Ctrl+D` / `Ctrl+Shift+K` / `Alt+↑↓` | Duplicate / delete / move lines |
| `Ctrl+Space` | Show suggestions |
| `Ctrl+N` / `Ctrl+Shift+N` | New file / new project |
| `Ctrl+Alt+N` | New window (launching rutex again does the same) |
| `Ctrl+Shift+E` / `Ctrl+Shift+O` / `F9` | Files / outline / toggle sidebar |
| `Ctrl+E` | Problems panel |
| `Ctrl++` / `Ctrl+-` / `Ctrl+0` / `Ctrl+wheel` | PDF zoom |
| `F11` / `Esc` | PDF fullscreen / back |
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
RUTEX_DEV=1 cargo run      # separate instance that runs alongside the installed one
```

## License

GPL-3.0-or-later. rutex links against Poppler, which is GPL-licensed.
