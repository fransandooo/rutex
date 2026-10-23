//! Searchable palette of math symbols. Clicking one inserts its command,
//! wrapped in $…$ when the cursor is not already inside a formula.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;

use crate::i18n::tr;

/// (command, glyph, search words)
type Sym = (&'static str, &'static str, &'static str);

const GREEK: &[Sym] = &[
    ("\\alpha", "α", "alpha"), ("\\beta", "β", "beta"), ("\\gamma", "γ", "gamma"), ("\\delta", "δ", "delta"),
    ("\\epsilon", "ϵ", "epsilon"), ("\\varepsilon", "ε", "epsilon var"), ("\\zeta", "ζ", "zeta"), ("\\eta", "η", "eta"),
    ("\\theta", "θ", "theta"), ("\\vartheta", "ϑ", "theta var"), ("\\iota", "ι", "iota"), ("\\kappa", "κ", "kappa"),
    ("\\lambda", "λ", "lambda"), ("\\mu", "μ", "mu"), ("\\nu", "ν", "nu"), ("\\xi", "ξ", "xi"), ("\\pi", "π", "pi"),
    ("\\varpi", "ϖ", "pi var"), ("\\rho", "ρ", "rho"), ("\\varrho", "ϱ", "rho var"), ("\\sigma", "σ", "sigma"),
    ("\\varsigma", "ς", "sigma var"), ("\\tau", "τ", "tau"), ("\\upsilon", "υ", "upsilon"), ("\\phi", "ϕ", "phi"),
    ("\\varphi", "φ", "phi var"), ("\\chi", "χ", "chi"), ("\\psi", "ψ", "psi"), ("\\omega", "ω", "omega"),
    ("\\Gamma", "Γ", "gamma capital"), ("\\Delta", "Δ", "delta capital"), ("\\Theta", "Θ", "theta capital"),
    ("\\Lambda", "Λ", "lambda capital"), ("\\Xi", "Ξ", "xi capital"), ("\\Pi", "Π", "pi capital"),
    ("\\Sigma", "Σ", "sigma capital"), ("\\Upsilon", "Υ", "upsilon capital"), ("\\Phi", "Φ", "phi capital"),
    ("\\Psi", "Ψ", "psi capital"), ("\\Omega", "Ω", "omega capital"),
];

const OPERATORS: &[Sym] = &[
    ("\\sum", "∑", "sum sigma"), ("\\prod", "∏", "product"), ("\\int", "∫", "integral"), ("\\iint", "∬", "double integral"),
    ("\\oint", "∮", "contour integral"), ("\\partial", "∂", "partial derivative"), ("\\nabla", "∇", "nabla gradient"),
    ("\\infty", "∞", "infinity"), ("\\pm", "±", "plus minus"), ("\\mp", "∓", "minus plus"), ("\\times", "×", "times cross multiply"),
    ("\\div", "÷", "divide"), ("\\cdot", "⋅", "dot multiply"), ("\\circ", "∘", "compose circle"), ("\\ast", "∗", "asterisk star"),
    ("\\star", "⋆", "star"), ("\\otimes", "⊗", "tensor product times circle"), ("\\oplus", "⊕", "direct sum plus circle"),
    ("\\bigcup", "⋃", "big union"), ("\\bigcap", "⋂", "big intersection"), ("\\sqrt{}", "√", "square root"),
    ("\\frac{}{}", "½", "fraction"), ("\\lim", "lim", "limit"), ("\\max", "max", "maximum"), ("\\min", "min", "minimum"),
    ("\\log", "log", "logarithm"), ("\\ln", "ln", "natural logarithm"), ("\\exp", "exp", "exponential"),
    ("\\sin", "sin", "sine"), ("\\cos", "cos", "cosine"), ("\\tan", "tan", "tangent"),
];

const RELATIONS: &[Sym] = &[
    ("\\leq", "≤", "less equal"), ("\\geq", "≥", "greater equal"), ("\\neq", "≠", "not equal"), ("\\approx", "≈", "approximately"),
    ("\\equiv", "≡", "equivalent identical"), ("\\sim", "∼", "similar tilde"), ("\\simeq", "≃", "similar equal"),
    ("\\cong", "≅", "congruent"), ("\\propto", "∝", "proportional"), ("\\ll", "≪", "much less"), ("\\gg", "≫", "much greater"),
    ("\\prec", "≺", "precedes"), ("\\succ", "≻", "succeeds"), ("\\perp", "⊥", "perpendicular"), ("\\parallel", "∥", "parallel"),
    ("\\mid", "∣", "divides mid"), ("\\models", "⊨", "models entails"), ("\\vdash", "⊢", "proves turnstile"),
    ("\\doteq", "≐", "dot equal"), ("\\coloneqq", "≔", "defined as colon equals mathtools"),
];

const ARROWS: &[Sym] = &[
    ("\\to", "→", "to right arrow"), ("\\gets", "←", "left arrow gets"), ("\\leftrightarrow", "↔", "left right arrow"),
    ("\\Rightarrow", "⇒", "implies right double arrow"), ("\\Leftarrow", "⇐", "left double arrow"),
    ("\\Leftrightarrow", "⇔", "iff equivalent double arrow"), ("\\mapsto", "↦", "maps to"), ("\\uparrow", "↑", "up arrow"),
    ("\\downarrow", "↓", "down arrow"), ("\\longrightarrow", "⟶", "long right arrow"), ("\\Longrightarrow", "⟹", "long implies"),
    ("\\iff", "⟺", "iff if and only if"), ("\\implies", "⟹", "implies amsmath"), ("\\hookrightarrow", "↪", "hook inclusion arrow"),
    ("\\rightharpoonup", "⇀", "harpoon weak convergence"), ("\\nearrow", "↗", "north east arrow"), ("\\searrow", "↘", "south east arrow"),
];

const SETS: &[Sym] = &[
    ("\\in", "∈", "in element of belongs"), ("\\notin", "∉", "not in"), ("\\ni", "∋", "contains as member"),
    ("\\subset", "⊂", "subset"), ("\\subseteq", "⊆", "subset equal"), ("\\supset", "⊃", "superset"), ("\\supseteq", "⊇", "superset equal"),
    ("\\cup", "∪", "union"), ("\\cap", "∩", "intersection"), ("\\setminus", "∖", "set minus difference"),
    ("\\emptyset", "∅", "empty set"), ("\\varnothing", "⌀", "empty set var"), ("\\forall", "∀", "for all"), ("\\exists", "∃", "exists"),
    ("\\nexists", "∄", "not exists"), ("\\neg", "¬", "not negation"), ("\\land", "∧", "and wedge"), ("\\lor", "∨", "or vee"),
    ("\\mathbb{R}", "ℝ", "real numbers"), ("\\mathbb{N}", "ℕ", "natural numbers"), ("\\mathbb{Z}", "ℤ", "integers"),
    ("\\mathbb{Q}", "ℚ", "rationals"), ("\\mathbb{C}", "ℂ", "complex numbers"), ("\\aleph", "ℵ", "aleph cardinal"),
];

const ACCENTS: &[Sym] = &[
    ("\\hat{}", "x̂", "hat circumflex"), ("\\bar{}", "x̄", "bar overline"), ("\\tilde{}", "x̃", "tilde"), ("\\vec{}", "x⃗", "vector arrow"),
    ("\\dot{}", "ẋ", "dot derivative"), ("\\ddot{}", "ẍ", "double dot second derivative"), ("\\overline{}", "x̅", "overline"),
    ("\\underline{}", "x̲", "underline"), ("\\widehat{}", "x̂", "wide hat"), ("\\mathbf{}", "𝐱", "bold"), ("\\mathcal{}", "𝒳", "calligraphic"),
    ("\\mathrm{}", "x", "roman upright"), ("\\boldsymbol{}", "𝛂", "bold symbol greek"), ("\\text{}", "T", "text in math"),
];

const DELIMITERS: &[Sym] = &[
    ("\\left( \\right)", "( )", "parentheses sized"), ("\\left[ \\right]", "[ ]", "brackets sized"), ("\\left\\{ \\right\\}", "{ }", "braces sized"),
    ("\\langle \\rangle", "⟨ ⟩", "angle brackets"), ("\\lvert \\rvert", "| |", "absolute value"), ("\\lVert \\rVert", "‖ ‖", "norm"),
    ("\\lfloor \\rfloor", "⌊ ⌋", "floor"), ("\\lceil \\rceil", "⌈ ⌉", "ceiling"), ("\\ldots", "…", "dots ellipsis"), ("\\cdots", "⋯", "centered dots"),
    ("\\vdots", "⋮", "vertical dots"), ("\\ddots", "⋱", "diagonal dots"), ("\\quad", "␣", "space quad"), ("\\prime", "′", "prime"),
    ("\\degree", "°", "degree gensymb"), ("\\hbar", "ℏ", "h bar planck"), ("\\ell", "ℓ", "ell script l"),
];

fn categories() -> [(&'static str, &'static [Sym]); 7] {
    [
        ("Greek letters", GREEK),
        ("Operators and functions", OPERATORS),
        ("Relations", RELATIONS),
        ("Arrows", ARROWS),
        ("Sets and logic", SETS),
        ("Accents and fonts", ACCENTS),
        ("Delimiters and dots", DELIMITERS),
    ]
}

/// Whether a symbol matches a search ("arrow", "\\sub", "ℝ").
pub fn matches(sym: &Sym, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    q.is_empty() || sym.0.to_lowercase().contains(&q) || sym.2.contains(&q) || sym.1 == query.trim()
}

thread_local! {
    static RECENT: RefCell<Vec<Sym>> = const { RefCell::new(Vec::new()) };
}

/// Opens the palette; `on_pick` receives the command to insert.
pub fn show(parent: &impl IsA<gtk::Widget>, on_pick: impl Fn(&'static str) + 'static) {
    let on_pick = Rc::new(on_pick);
    let search = gtk::SearchEntry::builder().placeholder_text(tr("Search symbols (arrow, subset, infinity…)")).build();
    let sections = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(14).margin_start(14).margin_end(14).margin_bottom(14).build();
    let dialog = adw::Dialog::builder().title(tr("Math symbols")).content_width(560).content_height(560).build();

    let mut flowboxes: Vec<(gtk::Box, gtk::FlowBox)> = Vec::new();
    let recent: Vec<Sym> = RECENT.with(|r| r.borrow().clone());
    let mut cats: Vec<(&str, Vec<Sym>)> = Vec::new();
    if !recent.is_empty() {
        cats.push(("Recent", recent));
    }
    cats.extend(categories().iter().map(|(t, s)| (*t, s.to_vec())));

    for (title, syms) in cats {
        let section = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).build();
        section.append(&gtk::Label::builder().label(tr(title)).xalign(0.0).css_classes(["heading"]).build());
        let flow = gtk::FlowBox::builder().selection_mode(gtk::SelectionMode::None).homogeneous(true).min_children_per_line(6).max_children_per_line(10).column_spacing(4).row_spacing(4).build();
        for sym in syms {
            let btn = gtk::Button::builder().label(sym.1).tooltip_text(format!("{}  —  {}", sym.0, sym.2)).css_classes(["symbol"]).build();
            let (cb, dlg) = (on_pick.clone(), dialog.clone());
            btn.connect_clicked(move |_| {
                RECENT.with(|r| {
                    let mut r = r.borrow_mut();
                    r.retain(|s| s.0 != sym.0);
                    r.insert(0, sym);
                    r.truncate(10);
                });
                dlg.close();
                cb(sym.0);
            });
            let child = gtk::FlowBoxChild::builder().child(&btn).build();
            // The symbol travels with the child, for filtering.
            unsafe { child.set_data("rutex-sym", sym) };
            flow.insert(&child, -1);
        }
        section.append(&flow);
        sections.append(&section);
        flowboxes.push((section, flow));
    }

    let fb = flowboxes.clone();
    search.connect_search_changed(move |e| {
        let q = e.text().to_string();
        for (section, flow) in &fb {
            let q2 = q.clone();
            flow.set_filter_func(move |child| unsafe { child.data::<Sym>("rutex-sym").is_some_and(|s| matches(s.as_ref(), &q2)) });
            let any = {
                let mut c = flow.first_child();
                let mut found = false;
                while let Some(w) = c {
                    if let Some(s) = unsafe { w.data::<Sym>("rutex-sym") }
                        && matches(unsafe { s.as_ref() }, &q)
                    {
                        found = true;
                        break;
                    }
                    c = w.next_sibling();
                }
                found
            };
            section.set_visible(any);
        }
    });
    // Enter inserts the first visible match.
    let (fb, cb, dlg) = (flowboxes, on_pick.clone(), dialog.clone());
    search.connect_activate(move |e| {
        let q = e.text().to_string();
        for (_, flow) in &fb {
            let mut c = flow.first_child();
            while let Some(w) = c {
                if let Some(s) = unsafe { w.data::<Sym>("rutex-sym") }
                    && matches(unsafe { s.as_ref() }, &q)
                {
                    let sym = unsafe { *s.as_ref() };
                    dlg.close();
                    cb(sym.0);
                    return;
                }
                c = w.next_sibling();
            }
        }
    });

    let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let top = gtk::Box::builder().margin_start(14).margin_end(14).margin_top(4).build();
    search.set_hexpand(true);
    top.append(&search);
    content.append(&top);
    content.append(&gtk::ScrolledWindow::builder().child(&sections).vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build());
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(&content));
    dialog.set_child(Some(&toolbar));
    dialog.present(Some(parent));
    search.grab_focus();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn searches_symbols() {
        let all: Vec<Sym> = categories().iter().flat_map(|(_, s)| s.iter().copied()).collect();
        let found = |q: &str| all.iter().filter(|s| matches(s, q)).map(|s| s.0).collect::<Vec<_>>();
        assert!(found("infinity").contains(&"\\infty"));
        assert!(found("subset").contains(&"\\subseteq"));
        assert!(found("\\alp").contains(&"\\alpha"));
        assert!(found("ℝ").contains(&"\\mathbb{R}"));
        assert!(all.len() > 150);
    }
}
