//! Traducciones. El texto de la interfaz está escrito en inglés en el código
//! (`tr("Open project")`) y aquí está su versión en español. El idioma se
//! elige en Preferencias; por defecto, inglés.

use std::collections::HashMap;
use std::fmt::Display;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, Ordering};

static SPANISH: AtomicBool = AtomicBool::new(false);

pub fn set_spanish(on: bool) {
    SPANISH.store(on, Ordering::Relaxed);
}

pub fn is_spanish() -> bool {
    SPANISH.load(Ordering::Relaxed)
}

/// Traduce un texto de la interfaz (si falta en la tabla, se queda en inglés).
pub fn tr(en: &'static str) -> &'static str {
    lookup(en, is_spanish())
}

fn lookup(en: &'static str, spanish: bool) -> &'static str {
    if spanish { ES.get(en).copied().unwrap_or(en) } else { en }
}

/// Como `tr`, pero sustituye cada `{}` por el siguiente argumento.
pub fn trf(en: &'static str, args: &[&dyn Display]) -> String {
    fill(tr(en), args)
}

fn fill(template: &str, args: &[&dyn Display]) -> String {
    let mut out = String::new();
    let mut args = args.iter();
    let mut rest = template;
    while let Some(i) = rest.find("{}") {
        out.push_str(&rest[..i]);
        match args.next() {
            Some(a) => out.push_str(&a.to_string()),
            None => out.push_str("{}"),
        }
        rest = &rest[i + 2..];
    }
    out.push_str(rest);
    out
}

/// 1.5 → "1.5" en inglés, "1,5" en español.
pub fn decimal(x: f64) -> String {
    let s = format!("{x:.1}");
    if is_spanish() { s.replace('.', ",") } else { s }
}

/// Separador de miles: "," en inglés, "." en español.
pub fn thousands_sep() -> char {
    if is_spanish() { '.' } else { ',' }
}

static ES: LazyLock<HashMap<&'static str, &'static str>> = LazyLock::new(|| TABLE.iter().copied().collect());

#[rustfmt::skip]
const TABLE: &[(&str, &str)] = &[
    ("1 error", "1 error"),
    ("1 warning", "1 aviso"),
    ("A LaTeX project you already have", "Un proyecto LaTeX que ya tengas"),
    ("A complete project with figures, plots and a bibliography", "Un proyecto completo con figuras, gráficas y bibliografía"),
    ("Appearance", "Apariencia"),
    ("Auto-close brackets", "Cerrar llaves automáticamente"),
    ("Auxiliary files removed", "Archivos auxiliares eliminados"),
    ("Back to the editor (Esc)", "Volver al editor (Esc)"),
    ("Bold (\\textbf)", "Negrita (\\textbf)"),
    ("Build failed", "Falló la compilación"),
    ("Cancel", "Cancelar"),
    ("Change…", "Cambiar…"),
    ("Clean auxiliary files", "Limpiar archivos auxiliares"),
    ("Click the margin icon to see the problems list", "Clic en el icono del margen para ver la lista de problemas"),
    ("Close (Ctrl+W)", "Cerrar (Ctrl+W)"),
    ("Close tab", "Cerrar pestaña"),
    ("Comment or uncomment", "Comentar o descomentar"),
    ("Compilation", "Compilación"),
    ("Compile (Ctrl+Enter)", "Compilar (Ctrl+Intro)"),
    ("Compile automatically", "Compilar automáticamente"),
    ("Compile now", "Compilar ahora"),
    ("Compiling…", "Compilando…"),
    ("Copy path", "Copiar ruta"),
    ("Could not create it: {}", "No se pudo crear: {}"),
    ("Could not create the project: {}", "No se pudo crear el proyecto: {}"),
    ("Could not move it to the trash: {}", "No se pudo mover a la papelera: {}"),
    ("Could not open the PDF:\n{}", "No se pudo abrir el PDF:\n{}"),
    ("Could not open {}: {}", "No se pudo abrir {}: {}"),
    ("Could not open {}:\n{}", "No se pudo abrir {}:\n{}"),
    ("Could not prepare the demo: {}", "No se pudo preparar la demo: {}"),
    ("Could not rename: {}", "No se pudo renombrar: {}"),
    ("Could not run latexmk: {}", "No se pudo lanzar latexmk: {}"),
    ("Could not save {}: {}", "No se pudo guardar {}: {}"),
    ("Create project", "Crear proyecto"),
    ("Create", "Crear"),
    ("Ctrl+, preferences  ·  Ctrl+? keyboard shortcuts", "Ctrl+, preferencias  ·  Ctrl+? atajos de teclado"),
    ("Dark PDF", "PDF en modo oscuro"),
    ("Dark", "Oscuro"),
    ("Delay after you stop typing (ms)", "Espera tras dejar de escribir (ms)"),
    ("Delete line", "Borrar línea"),
    ("Does not exist: {}", "No existe: {}"),
    ("Duplicate line", "Duplicar línea"),
    ("Editing", "Edición"),
    ("Editor font size", "Tamaño de letra del editor"),
    ("Files", "Archivos"),
    ("Find in file…", "Buscar en el archivo…"),
    ("Find", "Buscar"),
    ("Fit width (Ctrl+0)", "Ajustar al ancho (Ctrl+0)"),
    ("Fit width", "Ajustar al ancho"),
    ("Folder name", "Nombre de la carpeta"),
    ("From a template: article, thesis, notes, slides…", "Desde una plantilla: artículo, TFM, apuntes, presentación…"),
    ("Fullscreen PDF (F11)", "Ver el PDF en grande (F11)"),
    ("Go to line", "Ir a la línea"),
    ("Go", "Ir"),
    ("Include it with \\input{{}}", "Inclúyelo con \\input{{}}"),
    ("Invalid name", "Nombre no válido"),
    ("It compiles automatically while you write.\nCtrl+Enter compiles now.", "Se compilará automáticamente al escribir.\nCtrl+Intro para compilar ahora."),
    ("Italic (\\textit)", "Cursiva (\\textit)"),
    ("Jump from source to PDF", "Ir del código al PDF"),
    ("Keyboard shortcuts", "Atajos de teclado"),
    ("LaTeX editor with live compilation", "Editor LaTeX con compilación automática"),
    ("Light", "Claro"),
    ("Line number", "Número de línea"),
    ("Location", "Ubicación"),
    ("Main file", "Archivo principal"),
    ("Main file: {}", "Archivo principal: {}"),
    ("Menu", "Menú"),
    ("Move line", "Mover línea"),
    ("Move to trash", "Mover a la papelera"),
    ("Move to trash?", "¿Mover a la papelera?"),
    ("New file (Ctrl+N)", "Nuevo archivo (Ctrl+N)"),
    ("New file", "Nuevo archivo"),
    ("New file…", "Nuevo archivo…"),
    ("New folder", "Nueva carpeta"),
    ("New folder…", "Nueva carpeta…"),
    ("New name or path for “{}”", "Nuevo nombre o ruta para «{}»"),
    ("New project folder", "Carpeta del nuevo proyecto"),
    ("New project", "Nuevo proyecto"),
    ("New project…", "Nuevo proyecto…"),
    ("No PDF yet", "Sin PDF todavía"),
    ("No TeX", "Sin TeX"),
    ("No open files", "Ningún archivo abierto"),
    ("No sections yet.\nUse \\section{…}", "Sin secciones todavía.\nUsa \\section{…}"),
    ("OK", "Aceptar"),
    ("Omarchy (automatic)", "Omarchy (automático)"),
    ("Open PDF in external viewer", "Abrir PDF en visor externo"),
    ("Open folder", "Abrir carpeta"),
    ("Open project", "Abrir proyecto"),
    ("Open project…", "Abrir proyecto…"),
    ("Open the demo", "Abrir la demo"),
    ("Open", "Abrir"),
    ("Outline", "Esquema"),
    ("PDF saved to {}", "PDF guardado en {}"),
    ("Paints pages with the theme colors (on screen only)", "Pinta las páginas con los colores del tema (solo en pantalla)"),
    ("Path copied", "Ruta copiada"),
    ("Path inside the project", "Ruta dentro del proyecto"),
    ("Path inside the project. Without an extension it will be .tex.", "Ruta dentro del proyecto. Si no pones extensión, será .tex."),
    ("Pick one in the sidebar or press Ctrl+P", "Elige uno en la barra lateral o pulsa Ctrl+P"),
    ("Preferences", "Preferencias"),
    ("Problems and output (Ctrl+E)", "Problemas y salida (Ctrl+E)"),
    ("Problems panel", "Panel de problemas"),
    ("Problems", "Problemas"),
    ("Project location", "Ubicación del proyecto"),
    ("Project", "Proyecto"),
    ("Quick open", "Apertura rápida de archivos"),
    ("Ready", "Listo"),
    ("Recent", "Recientes"),
    ("Redo", "Rehacer"),
    ("Remove from recent", "Quitar de recientes"),
    ("Rename", "Renombrar"),
    ("Rename…", "Renombrar…"),
    ("Save PDF to the project folder", "Guardar PDF en la carpeta del proyecto"),
    ("Save and compile", "Guardar y compilar"),
    ("Search files…  (@ for sections)", "Buscar archivo…  (@ para secciones)"),
    ("Show files", "Mostrar archivos"),
    ("Show in file manager", "Mostrar en el gestor de archivos"),
    ("Show outline", "Mostrar esquema"),
    ("Show suggestions", "Mostrar sugerencias"),
    ("Sidebar (F9)", "Barra lateral (F9)"),
    ("SyncTeX could not find that line in the PDF", "SyncTeX no encontró esa línea en el PDF"),
    ("SyncTeX could not find that position", "SyncTeX no encontró esa posición"),
    ("System", "Sistema"),
    ("Template", "Plantilla"),
    ("That folder already exists and is not empty. Pick another name or use “Open folder”.", "Esa carpeta ya existe y no está vacía. Elige otro nombre o usa «Abrir carpeta»."),
    ("The folder has no .tex files", "La carpeta no contiene archivos .tex"),
    ("The main file must be a .tex file", "El archivo principal debe ser un .tex"),
    ("Theme", "Tema"),
    ("There is no PDF yet", "Todavía no hay PDF"),
    ("Toggle sidebar", "Mostrar u ocultar la barra lateral"),
    ("Try the demo", "Ver la demo"),
    ("Type a valid folder name (without “/”).", "Escribe un nombre de carpeta válido (sin «/»)."),
    ("Undo", "Deshacer"),
    ("Use as main file", "Usar como archivo principal"),
    ("Use this file as main", "Usar este archivo como principal"),
    ("Vim mode", "Modo Vim"),
    ("When off, files are still saved but only compile with Ctrl+S or Ctrl+Enter", "Si lo desactivas, se guarda igual pero solo compila con Ctrl+S o Ctrl+Intro"),
    ("Wrap long lines", "Ajustar líneas largas"),
    ("You removed the main file: pick another with right click → Use as main file", "Has borrado el archivo principal: elige otro con clic derecho → Usar como archivo principal"),
    ("Zoom in (Ctrl++)", "Acercar (Ctrl++)"),
    ("Zoom in", "Acercar"),
    ("Zoom out (Ctrl+-)", "Alejar (Ctrl+-)"),
    ("Zoom out", "Alejar"),
    ("latexmk output", "Salida de latexmk"),
    ("line {}", "línea {}"),
    ("{ [ ( $ and \\end{…} when pressing Enter after \\begin{…}", "{ [ ( $ y \\end{…} al pulsar Intro tras \\begin{…}"),
    ("{} already exists", "Ya existe {}"),
    ("{} errors", "{} errores"),
    ("{} had changed outside rutex; it was overwritten", "{} había cambiado fuera de rutex; se ha sobrescrito"),
    ("{} is not UTF-8", "{} no está en UTF-8"),
    ("{} warnings", "{} avisos"),
    ("{} words", "{} palabras"),
    ("“Omarchy” follows your system theme and changes as soon as you change it", "«Omarchy» sigue el tema del sistema y cambia en cuanto lo cambias tú"),
    ("“{}” will be moved to the trash. You can restore it from your file manager.", "«{}» se moverá a la papelera. Podrás recuperarlo desde tu gestor de archivos."),    (".bib file (biblatex)", "Archivo .bib (biblatex)"),
    ("A set $A \\subseteq \\R$ is bounded if there is $M > 0$ with $|a| \\le M$ for every $a \\in A$.", "Un conjunto $A \\subseteq \\R$ está acotado si existe $M > 0$ con $|a| \\le M$ para todo $a \\in A$."),
    ("An idea", "Una idea"),
    ("Another idea", "Otra idea"),
    ("Article", "Artículo"),
    ("Author", "Autor"),
    ("Bar", "Barra"),
    ("Beamer slides", "Diapositivas con Beamer"),
    ("Bibliography (BibTeX)", "Bibliografía (BibTeX)"),
    ("Bibliography (biblatex)", "Bibliografía (biblatex)"),
    ("Bibliography style", "Estilo bibliográfico"),
    ("Bold", "Negrita"),
    ("Bold math", "Negrita matemática"),
    ("Calligraphic", "Caligráfica"),
    ("Center", "Centrar"),
    ("Chapter", "Capítulo"),
    ("Chapters in separate files, biblatex bibliography and figures", "Capítulos en archivos separados, bibliografía con biblatex y figuras"),
    ("Citation", "Cita"),
    ("Citation (natbib)", "Cita (natbib)"),
    ("Class notes with theorems, definitions and examples", "Notas de clase con teoremas, definiciones y ejemplos"),
    ("Close the open environment", "Cerrar entorno abierto"),
    ("Conclusions", "Conclusiones"),
    ("Contents", "Índice"),
    ("Context and motivation of the work", "Contexto y motivación del trabajo"),
    ("Date", "Fecha"),
    ("Defined in the project", "Definido en el proyecto"),
    ("Definition", "Definición"),
    ("Document class", "Clase de documento"),
    ("Document structure", "Estructura del documento"),
    ("Emphasis", "Énfasis"),
    ("Empty", "Vacío"),
    ("Environment", "Entorno"),
    ("Equation reference", "Referencia a ecuación"),
    ("Every non-empty subset of $\\R$ that is bounded above has a supremum.", "Todo conjunto no vacío y acotado superiormente de $\\R$ tiene supremo."),
    ("Example", "Ejemplo"),
    ("Figure/table caption", "Pie de figura/tabla"),
    ("First slide", "Primera diapositiva"),
    ("Footnote", "Nota al pie"),
    ("Fraction", "Fracción"),
    ("Future work", "Trabajo futuro"),
    ("Hat", "Acento circunflejo"),
    ("Hello.", "Hola."),
    ("Horizontal space", "Espacio horizontal"),
    ("Image", "Imagen"),
    ("Include chapter", "Incluir capítulo"),
    ("Include file", "Incluir archivo"),
    ("Introduction", "Introducción"),
    ("Italic", "Cursiva"),
    ("Just the essentials", "Solo lo imprescindible"),
    ("Label", "Etiqueta"),
    ("Lecture notes", "Apuntes"),
    ("Lemma", "Lema"),
    ("Limit", "Límite"),
    ("Line width", "Ancho de línea"),
    ("Link", "Enlace"),
    ("List item", "Elemento de lista"),
    ("List of figures", "Índice de figuras"),
    ("List of tables", "Índice de tablas"),
    ("Merged cell", "Celda combinada"),
    ("Methodology", "Metodología"),
    ("Monospace", "Monoespaciado"),
    ("Name Surname", "Nombre Apellido"),
    ("Named reference", "Referencia con nombre"),
    ("New command", "Nuevo comando"),
    ("New page", "Nueva página"),
    ("New page (flushes floats)", "Nueva página (vacía flotantes)"),
    ("No indent", "Sin sangría"),
    ("Number sets (ℝ, ℕ…)", "Conjuntos (ℝ, ℕ…)"),
    ("Objectives", "Objetivos"),
    ("On Debian/Ubuntu: texlive-full or texlive-latex-extra + latexmk. On Fedora: texlive-scheme-medium + latexmk.", "En Debian/Ubuntu: texlive-full o texlive-latex-extra + latexmk. En Fedora: texlive-scheme-medium + latexmk."),
    ("Operator", "Operador"),
    ("Package", "Paquete"),
    ("Page of a label", "Página de una etiqueta"),
    ("Paragraph", "Párrafo"),
    ("Parenthetical citation", "Cita entre paréntesis"),
    ("Part", "Parte"),
    ("Presentation", "Presentación"),
    ("Product", "Productorio"),
    ("Reference", "Referencia"),
    ("Reference (cleveref)", "Referencia (cleveref)"),
    ("Remark", "Observación"),
    ("Results", "Resultados"),
    ("Roman", "Romana"),
    ("Section", "Sección"),
    ("Simple document with sections, formulas and figures", "Documento sencillo con secciones, fórmulas y figuras"),
    ("Size", "Tamaño"),
    ("Sized brackets", "Corchetes ajustados"),
    ("Sized parentheses", "Paréntesis ajustados"),
    ("Small caps", "Versalitas"),
    ("Space", "Espacio"),
    ("Square root", "Raíz"),
    ("Start writing here. Formulas look like this:", "Empieza a escribir aquí. Las fórmulas van así:"),
    ("State of the art", "Estado del arte"),
    ("Subsection", "Subsección"),
    ("Subsubsection", "Subsubsección"),
    ("Sum", "Sumatorio"),
    ("Summary of the work.", "Resumen del trabajo."),
    ("Table of contents", "Índice"),
    ("Table rule", "Línea de tabla"),
    ("Text in math", "Texto en fórmula"),
    ("Text width", "Ancho del texto"),
    ("Textual citation", "Cita en el texto"),
    ("Textual citation (natbib)", "Cita textual (natbib)"),
    ("Theorem", "Teorema"),
    ("Thesis (TFM / TFG / PhD)", "TFM / TFG / Tesis"),
    ("This is the completeness axiom.", "Es el axioma del supremo."),
    ("Title", "Título"),
    ("Title page", "Portada"),
    ("Today's date", "Fecha de hoy"),
    ("Topic 1", "Tema 1"),
    ("Underline", "Subrayado"),
    ("Vertical space", "Espacio vertical"),
    ("chapters", "capitulos"),
    ("conclusions", "conclusiones"),
    ("figures", "figuras"),
    ("introduction", "introduccion"),
    ("latexmk was not found. Install TeX Live, for example on Arch:", "No se encontró latexmk. Instala TeX Live, por ejemplo en Arch:"),
    ("methodology", "metodologia"),
    ("my-project", "mi-proyecto"),
    ("references", "referencias"),
    ("results", "resultados"),
    ("state-of-the-art", "estado-del-arte"),
    ("the build thread failed", "el hilo de compilación falló"),
    ("this file", "este archivo"),
    ("Back to where you were", "Volver adonde estabas"),
    ("That part is generated by LaTeX (table of contents, bibliography…) and has no source line", "Esa parte la genera LaTeX (índice, bibliografía…) y no tiene línea en el código"),
    ("chap", "cap"),
    ("Check spelling", "Revisar la ortografía"),
    ("Right-click a word for suggestions", "Clic derecho en una palabra para ver sugerencias"),
    ("Same as the document (babel)", "El del documento (babel)"),
    ("Spelling language", "Idioma de la ortografía"),
    ("No dictionary installed for {} (install hunspell-{})", "No hay diccionario instalado para {} (instala hunspell-{})"),
    ("just now", "ahora mismo"),
    ("{} min ago", "hace {} min"),
    ("{} h ago", "hace {} h"),
    ("today, {}", "hoy, {}"),
    ("yesterday, {}", "ayer, {}"),
    ("No versions yet.\nThey are saved automatically while you work.", "Todavía no hay versiones.\nSe guardan solas mientras trabajas."),
    ("Restore this version", "Restaurar esta versión"),
    ("Save a version now", "Guardar una versión ahora"),
    ("Version history", "Historial de versiones"),
    ("Version history…", "Historial de versiones…"),
    ("Save a version…", "Guardar una versión…"),
    ("Save a version", "Guardar una versión"),
    ("Automatic", "Automática"),
    ("{} files", "{} archivos"),
    ("Comparing…", "Comparando…"),
    ("Restore this version?", "¿Restaurar esta versión?"),
    ("The project files will be replaced by this version. Your current state is saved first, so you can go back to it from this same list.", "Los archivos del proyecto se sustituirán por esta versión. Antes se guarda tu estado actual, así que podrás volver a él desde esta misma lista."),
    ("Restore", "Restaurar"),
    ("Before restoring “{}”", "Antes de restaurar «{}»"),
    ("e.g. Sent to my supervisor", "p. ej. Enviado al tutor"),
    ("Give it a name to find it easily later.", "Ponle un nombre para encontrarla fácilmente."),
    ("Save", "Guardar"),
    ("Unnamed version", "Versión sin nombre"),
    ("No differences: this version is identical to your current files.", "Sin diferencias: esta versión es idéntica a tus archivos actuales."),
    ("Red: only in this version · Green: added since then", "Rojo: solo en esta versión · Verde: añadido desde entonces"),
    ("Install git to use the version history", "Instala git para usar el historial de versiones"),
    ("Version restored. Your previous state is saved in the history.", "Versión restaurada. Tu estado anterior queda guardado en el historial."),
    ("Version saved", "Versión guardada"),
    ("Ctrl+click to go there", "Ctrl+clic para ir allí"),
    ("No \\label{{}} in the project", "No hay ningún \\label{{}} en el proyecto"),
    ("{} is not in any .bib file", "{} no está en ningún archivo .bib"),
    ("Search", "Buscar"),
    ("Search in project…", "Buscar en el proyecto…"),
    ("Match case", "Distinguir mayúsculas"),
    ("Whole word", "Palabra completa"),
    ("Replace with…", "Reemplazar por…"),
    ("Replace all (a version is saved first)", "Reemplazar todo (antes se guarda una versión)"),
    ("No results", "Sin resultados"),
    ("{} results in {} files", "{} resultados en {} archivos"),
    ("Before replacing “{}”", "Antes de reemplazar «{}»"),
    ("Replaced {} occurrences in {} files", "Reemplazadas {} apariciones en {} archivos"),
    ("Could not replace: {}", "No se pudo reemplazar: {}"),
    ("Find in project", "Buscar en el proyecto"),
    ("Check before submitting…", "Revisar antes de entregar…"),
    ("Check before submitting", "Revisar antes de entregar"),
    ("Ready to submit", "Listo para entregar"),
    ("No undefined references, unused labels or citations, margin overflows or to-do notes.", "Ni referencias sin definir, ni etiquetas o citas sin usar, ni texto que se sale del margen, ni notas pendientes."),
    ("Based on the last build. Click an item to go to it.", "Según la última compilación. Pulsa un elemento para ir a él."),
    ("Undefined references and citations", "Referencias y citas sin definir"),
    ("Labels never referenced", "Etiquetas que nunca se referencian"),
    ("Labels defined more than once", "Etiquetas definidas más de una vez"),
    ("Bibliography entries never cited", "Entradas de la bibliografía sin citar"),
    ("Text sticking into the margin", "Texto que se sale del margen"),
    ("To-do notes", "Notas pendientes"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translates_and_formats() {
        // Sin tocar el idioma global: los tests corren en paralelo.
        assert_eq!(lookup("Open project", false), "Open project");
        assert_eq!(lookup("Open project", true), "Abrir proyecto");
        assert_eq!(fill(lookup("{} words", true), &[&12]), "12 palabras");
        assert_eq!(fill(lookup("Include it with \\input{{}}", true), &[&"cap/a"]), "Inclúyelo con \\input{cap/a}");
        assert_eq!(lookup("Not in the table", true), "Not in the table");
    }

    #[test]
    fn no_duplicate_keys() {
        let mut keys: Vec<_> = TABLE.iter().map(|(k, _)| *k).collect();
        keys.sort();
        let before = keys.len();
        keys.dedup();
        assert_eq!(before, keys.len());
    }
}
