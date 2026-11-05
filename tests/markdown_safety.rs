//! Markdown-safety tests: literal document text must never be misread as
//! Markdown syntax. Random strings full of Markdown-special characters are
//! written as LaTeX, converted, parsed back with a real CommonMark parser
//! (pulldown-cmark, with GFM tables), and the visible text must equal the
//! original in every context (paragraphs, emphasis, headings, list items,
//! table cells, quotes, line starts after hard breaks, ...).

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

const PIECES: &[&str] = &[
    "a", "b", "c", "x", "y", "z", "0", "1", "9", " ", "*", "_", "[", "]", "(", ")", "<", ">",
    "#", "!", "+", "=", "|", "~", "&", ";", ":", ".", ",", "?", "/", "@", "^", "{", "}", "$",
    "%", "\\", "\"", "-", " - ", "1.", "> ", "* ", "&amp;", "&#35;", "](", "`", "``", "**",
    "__", "~~", "<b>", "</b>", "<!--", "http://x.y", "[^1]", "![", "\\\\",
];

/// Write text as LaTeX that typesets exactly that text.
fn to_latex(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\textbackslash{}"),
            '{' => out.push_str("\\{"),
            '}' => out.push_str("\\}"),
            '$' => out.push_str("\\$"),
            '&' => out.push_str("\\&"),
            '%' => out.push_str("\\%"),
            '#' => out.push_str("\\#"),
            '_' => out.push_str("\\_"),
            '^' => out.push_str("\\textasciicircum{}"),
            '~' => out.push_str("\\textasciitilde{}"),
            '<' => out.push_str("\\textless{}"),
            '>' => out.push_str("\\textgreater{}"),
            '|' => out.push_str("\\textbar{}"),
            '`' => out.push_str("\\textasciigrave{}"),
            '"' => out.push_str("\\textquotedbl{}"),
            _ => out.push(ch),
        }
    }
    out
}

fn normalize(text: &str) -> String {
    text.replace('\u{a0}', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn random_case(rng: &mut Rng) -> String {
    let len = 1 + rng.below(12);
    let mut text = String::new();
    for _ in 0..len {
        text.push_str(PIECES[rng.below(PIECES.len())]);
    }
    // `--` is a TeX ligature (en dash); keep hyphens single
    while text.contains("--") {
        text = text.replace("--", "-x-");
    }
    let text = normalize(&text);
    if text.is_empty() {
        "a".to_string()
    } else {
        text
    }
}

/// What a rendered Markdown document looks like to a reader.
struct Rendered {
    text: String,
    paragraphs: usize,
    headings: usize,
    list_items: usize,
    quotes: usize,
    code_blocks: usize,
    table_cells: Vec<String>,
    strong: bool,
    emph: bool,
    links: usize,
    unexpected_html: Vec<String>,
}

const ALLOWED_HTML: &[&str] = &[
    "<em>", "</em>", "<strong>", "</strong>", "<del>", "</del>", "<u>", "</u>", "<br>",
    "<!-- -->",
];

fn render(markdown: &str) -> Rendered {
    let mut r = Rendered {
        text: String::new(),
        paragraphs: 0,
        headings: 0,
        list_items: 0,
        quotes: 0,
        code_blocks: 0,
        table_cells: Vec::new(),
        strong: false,
        emph: false,
        links: 0,
        unexpected_html: Vec::new(),
    };
    let mut cell: Option<String> = None;
    let options = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_FOOTNOTES;
    for event in Parser::new_ext(markdown, options) {
        let mut push = |r: &mut Rendered, s: &str| {
            r.text.push_str(s);
            if let Some(c) = cell.as_mut() {
                c.push_str(s);
            }
        };
        match event {
            Event::Start(Tag::Paragraph) => r.paragraphs += 1,
            Event::Start(Tag::Heading { .. }) => r.headings += 1,
            Event::Start(Tag::Item) => r.list_items += 1,
            Event::Start(Tag::BlockQuote(_)) => r.quotes += 1,
            Event::Start(Tag::CodeBlock(_)) => r.code_blocks += 1,
            Event::Start(Tag::Strong) => r.strong = true,
            Event::Start(Tag::Emphasis) => r.emph = true,
            Event::Start(Tag::Link { .. }) | Event::Start(Tag::Image { .. }) => r.links += 1,
            Event::Start(Tag::TableCell) => cell = Some(String::new()),
            Event::End(TagEnd::TableCell) => {
                if let Some(c) = cell.take() {
                    r.table_cells.push(c);
                }
            }
            Event::End(TagEnd::Paragraph) | Event::End(TagEnd::Heading(_)) => push(&mut r, " "),
            Event::Text(t) | Event::Code(t) => push(&mut r, &t),
            Event::SoftBreak | Event::HardBreak => push(&mut r, " "),
            Event::InlineHtml(h) | Event::Html(h) => {
                let tag = h.trim();
                match tag {
                    "<strong>" => r.strong = true,
                    "<em>" => r.emph = true,
                    _ => {}
                }
                if !ALLOWED_HTML.contains(&tag) {
                    r.unexpected_html.push(tag.to_string());
                }
            }
            Event::FootnoteReference(f) => push(&mut r, &format!("[^{}]", f)),
            _ => {}
        }
    }
    r
}

fn convert(latex: &str) -> String {
    texed::convert_with(
        latex,
        &texed::Options {
            no_frontmatter: true,
            ..Default::default()
        },
    )
    .expect("conversion succeeds")
    .markdown
}

/// Run `cases` random strings through `wrap` and check them with `check`.
fn check_context(name: &str, seed: u64, wrap: fn(&str) -> String, check: fn(&str, &Rendered) -> bool) {
    check_context_links(name, seed, 0, wrap, check)
}

fn check_context_links(
    name: &str,
    seed: u64,
    links: usize,
    wrap: fn(&str) -> String,
    check: fn(&str, &Rendered) -> bool,
) {
    let mut rng = Rng(seed);
    let mut failures = Vec::new();
    for _ in 0..400 {
        let case = random_case(&mut rng);
        let markdown = convert(&wrap(&to_latex(&case)));
        let rendered = render(&markdown);
        let structural_ok = rendered.unexpected_html.is_empty()
            && rendered.links == links
            && rendered.code_blocks == 0;
        if !structural_ok || !check(&case, &rendered) {
            failures.push(format!(
                "  text {:?}\n  markdown {:?}\n  seen {:?} (html {:?}, links {})",
                case,
                markdown,
                normalize(&rendered.text),
                rendered.unexpected_html,
                rendered.links
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "[{}] {} case(s) did not round-trip:\n{}",
        name,
        failures.len(),
        failures.iter().take(5).cloned().collect::<Vec<_>>().join("\n")
    );
}

fn same_text(case: &str, r: &Rendered) -> bool {
    normalize(&r.text) == *case
}

#[test]
fn plain_paragraphs() {
    check_context("paragraph", 1, |c| c.to_string(), |c, r| {
        same_text(c, r) && r.paragraphs == 1 && r.headings == 0 && r.list_items == 0 && r.quotes == 0
    });
}

#[test]
fn bold_text() {
    check_context("textbf", 2, |c| format!("\\textbf{{{}}}", c), |c, r| same_text(c, r) && r.strong);
}

#[test]
fn emphasized_text() {
    check_context("emph", 3, |c| format!("\\emph{{{}}}", c), |c, r| same_text(c, r) && r.emph);
}

#[test]
fn headings() {
    check_context("section", 4, |c| format!("\\section*{{{}}}", c), |c, r| {
        same_text(c, r) && r.headings == 1 && r.paragraphs == 0
    });
}

#[test]
fn list_items() {
    check_context("item", 5, |c| format!("\\begin{{itemize}}\\item{{}} {}\\end{{itemize}}", c), |c, r| {
        same_text(c, r) && r.list_items == 1
    });
}

#[test]
fn table_cells() {
    check_context(
        "table",
        6,
        |c| format!("\\begin{{tabular}}{{ll}} h & {} \\\\ x & y \\\\ \\end{{tabular}}", c),
        |c, r| r.table_cells.len() == 4 && normalize(&r.table_cells[1]) == *c,
    );
}

#[test]
fn mid_sentence() {
    check_context("mid-sentence", 7, |c| format!("word {} word", c), |c, r| {
        normalize(&r.text) == format!("word {} word", c) && r.paragraphs == 1
    });
}

#[test]
fn line_start_after_hard_break() {
    check_context("after-linebreak", 8, |c| format!("lead \\\\ {}", c), |c, r| {
        normalize(&r.text) == format!("lead {}", c) && r.paragraphs == 1 && r.list_items == 0
    });
}

#[test]
fn quotations() {
    check_context("quote", 9, |c| format!("\\begin{{quote}}{}\\end{{quote}}", c), |c, r| {
        same_text(c, r) && r.quotes == 1
    });
}

#[test]
fn footnote_text() {
    check_context("footnote", 10, |c| format!("x\\footnote{{{}}}", c), |c, r| {
        // reference + definition; definition text must be intact
        normalize(&r.text).ends_with(c)
    });
}

#[test]
fn captions_and_link_text() {
    check_context_links(
        "href",
        11,
        1,
        |c| format!("\\href{{https://example.com/a(b) c}}{{{}}}", c),
        |c, r| same_text(c, r),
    );
}
