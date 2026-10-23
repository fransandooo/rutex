//! Lanzamiento de latexmk (y de cualquier otro proceso externo) sin dejar
//! procesos huérfanos:
//! - cada hijo va en su propio grupo de procesos, así matamos también los
//!   pdflatex/biber que lance latexmk;
//! - `PR_SET_PDEATHSIG` hace que el kernel mate al hijo si rutex muere de
//!   golpe. Los hijos se crean siempre desde el hilo principal, que vive tanto
//!   como la aplicación (PDEATHSIG se dispara cuando muere el hilo que hizo el fork);
//! - un registro global permite matar todo al salir.

use std::fs;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use gtk::gio;

use crate::logparse::{self, Issue};
use crate::project::{self, Project};

static RUNNING: Mutex<Vec<i32>> = Mutex::new(Vec::new());

/// Crea un `Command` que muere con nosotros y forma su propio grupo.
pub fn command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    cmd.stdin(Stdio::null());
    let parent = std::process::id() as i32;
    unsafe {
        cmd.pre_exec(move || {
            libc::setpgid(0, 0);
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
            // Si el padre murió entre el fork y el prctl, salimos ya.
            if libc::getppid() != parent {
                libc::_exit(1);
            }
            Ok(())
        });
    }
    cmd
}

/// Mata el grupo de procesos completo.
pub fn kill(pid: i32) {
    unsafe {
        libc::kill(-pid, libc::SIGKILL);
    }
}

pub fn kill_all() {
    let pids = std::mem::take(&mut *RUNNING.lock().unwrap());
    for pid in pids {
        kill(pid);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engine {
    PdfLatex,
    XeLatex,
    LuaLatex,
    /// Hay un latexmkrc en el proyecto y ningún comentario mágico: que decida él.
    FromRc,
}

impl Engine {
    pub fn detect(root: &Path, main_src: &str) -> Engine {
        if let Some(p) = project::magic_comment(main_src, "program") {
            let p = p.to_ascii_lowercase();
            if p.contains("xelatex") {
                return Engine::XeLatex;
            } else if p.contains("lualatex") {
                return Engine::LuaLatex;
            } else if p.contains("pdflatex") {
                return Engine::PdfLatex;
            }
        }
        if root.join("latexmkrc").exists() || root.join(".latexmkrc").exists() {
            return Engine::FromRc;
        }
        let needs_unicode_engine = main_src.lines().map(project::strip_comment).any(|l| {
            l.contains("{fontspec}") || l.contains("{unicode-math}") || l.contains("{polyglossia}")
        });
        if needs_unicode_engine { Engine::XeLatex } else { Engine::PdfLatex }
    }

    fn flag(self) -> Option<&'static str> {
        match self {
            Engine::PdfLatex => Some("-pdf"),
            Engine::XeLatex => Some("-pdfxe"),
            Engine::LuaLatex => Some("-pdflua"),
            Engine::FromRc => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Engine::PdfLatex => "pdfLaTeX",
            Engine::XeLatex => "XeLaTeX",
            Engine::LuaLatex => "LuaLaTeX",
            Engine::FromRc => "latexmkrc",
        }
    }
}

pub struct Outcome {
    pub ok: bool,
    pub issues: Vec<Issue>,
    /// Salida de latexmk (para cuando falla antes de llegar a TeX).
    pub output: String,
    pub pdf: Option<(PathBuf, SystemTime)>,
    pub elapsed: Duration,
}

pub struct Running {
    pub pid: i32,
    pub engine: Engine,
    child: Child,
    started: Instant,
    pdf: PathBuf,
    log: PathBuf,
}

pub fn spawn(project: &Project) -> std::io::Result<Running> {
    let main_src = fs::read_to_string(project.root.join(&project.main)).unwrap_or_default();
    let engine = Engine::detect(&project.root, &main_src);

    fs::create_dir_all(&project.build_dir)?;
    // TeX no crea subcarpetas: replicamos las del proyecto para los .aux de \include.
    for f in &project.files {
        if let Some(parent) = f.parent().filter(|p| !p.as_os_str().is_empty()) {
            let _ = fs::create_dir_all(project.build_dir.join(parent));
        }
    }

    let mut cmd = command("latexmk");
    cmd.current_dir(&project.root)
        .env("max_print_line", "10000")
        .env("error_line", "254")
        .env("half_error_line", "238")
        .arg("-interaction=nonstopmode")
        .arg("-file-line-error")
        .arg("-synctex=1")
        .arg("-f")
        .arg(format!("-outdir={}", project.build_dir.display()));
    if let Some(flag) = engine.flag() {
        cmd.arg(flag);
    }
    cmd.arg(&project.main).stdout(Stdio::piped()).stderr(Stdio::piped());

    let child = cmd.spawn()?;
    let pid = child.id() as i32;
    RUNNING.lock().unwrap().push(pid);
    Ok(Running { pid, engine, child, started: Instant::now(), pdf: project.pdf_path(), log: project.log_path() })
}

impl Running {
    /// Espera en un hilo del pool de GIO y parsea el log allí mismo.
    pub async fn wait(self) -> Outcome {
        let Running { pid, child, started, pdf, log, .. } = self;
        let result = gio::spawn_blocking(move || {
            let out = child.wait_with_output();
            let issues = fs::read(&log).map(|b| logparse::parse(&String::from_utf8_lossy(&b))).unwrap_or_default();
            (out, issues)
        })
        .await;
        RUNNING.lock().unwrap().retain(|p| *p != pid);

        let (out, issues) = match result {
            Ok(r) => r,
            Err(_) => (Err(std::io::Error::other("el hilo de compilación falló")), Vec::new()),
        };
        let (ok, output) = match out {
            Ok(o) => {
                let mut text = String::from_utf8_lossy(&o.stdout).into_owned();
                text.push_str(&String::from_utf8_lossy(&o.stderr));
                (o.status.success(), tail(&text, 300))
            }
            Err(e) => (false, e.to_string()),
        };
        let pdf = fs::metadata(&pdf).and_then(|m| m.modified()).ok().map(|t| (pdf, t));
        Outcome { ok, issues, output, pdf, elapsed: started.elapsed() }
    }
}

fn tail(text: &str, max_lines: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(max_lines)..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_detection() {
        let root = Path::new("/nonexistent");
        assert_eq!(Engine::detect(root, "\\documentclass{article}"), Engine::PdfLatex);
        assert_eq!(Engine::detect(root, "\\usepackage{fontspec}"), Engine::XeLatex);
        assert_eq!(Engine::detect(root, "% \\usepackage{fontspec}"), Engine::PdfLatex);
        assert_eq!(Engine::detect(root, "% !TEX program = lualatex\n\\usepackage{fontspec}"), Engine::LuaLatex);
    }
}
