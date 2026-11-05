use std::collections::HashMap;

/// State management for LaTeX parsing with cross-reference resolution
#[derive(Debug, Clone)]
pub struct ParserState {
    /// Label to reference mapping (label -> (type, number, title))
    pub labels: HashMap<String, LabelInfo>,
    
    /// Footnote counter
    pub footnote_counter: usize,
    
    /// Footnote marks waiting for text
    pub footnote_marks: HashMap<usize, String>,
    
    /// Footnote texts waiting for marks
    pub footnote_texts: HashMap<usize, String>,
    
    /// Figure counter
    pub figure_counter: usize,
    
    /// Table counter
    pub table_counter: usize,
    
    /// Equation counter
    pub equation_counter: usize,
    
    /// Section counters (by level)
    pub section_counters: Vec<usize>,
    
    /// Current section number as dotted string
    pub current_section_number: String,

    /// Level of the most recent sectioning command
    pub current_level: usize,
    
    /// Macro definitions (name -> (params, body))
    pub macros: HashMap<String, MacroDef>,

    /// Theorem-like environment definitions
    pub theorem_envs: HashMap<String, TheoremEnvDef>,

    /// User-defined environments (\newenvironment)
    pub environments: HashMap<String, EnvironmentDef>,

    /// Theorem-like counters
    pub theorem_counters: HashMap<String, usize>,
    
    /// Toggle states for conditional processing
    pub toggles: HashMap<String, bool>,
    
    /// File contents cache
    pub file_contents: HashMap<String, String>,
    
    /// Included files (for cycle detection)
    pub included_files: Vec<String>,
    
    /// Current language
    pub current_language: Option<String>,
    
    /// In list item flag
    pub in_list_item: bool,
    
    /// In table cell flag
    pub in_table_cell: bool,
    
    /// Metadata fields
    pub metadata: HashMap<String, String>,
    
    /// Bibliography entries
    pub bibliography: Vec<String>,

    /// Citation style
    pub citation_style: CitationStyle,

    /// Conversion diagnostics (unknown commands, unresolved refs, ...)
    pub warnings: Vec<String>,

    /// After `\appendix`: top-level sectioning is numbered A, B, C, ...
    pub appendix: bool,

    /// Document has chapters (book/report class, or a `\chapter` was seen):
    /// figures, tables and equations are then numbered per chapter.
    pub has_chapters: bool,

    /// `\numberwithin{counter}{parent}` overrides (counter -> parent).
    pub numberwithin: HashMap<String, String>,

    /// `\newif\ifname` booleans (name without the `if` prefix).
    pub conditionals: HashMap<String, bool>,

    /// `\let\new\old` aliases of built-in commands (new -> old).
    pub aliases: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct LabelInfo {
    pub label_type: LabelType,
    pub number: String,
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LabelType {
    Section,
    Figure,
    Table,
    Equation,
    Theorem,
    Lemma,
    Definition,
    Other(String),
}

#[derive(Debug, Clone, Default)]
pub struct MacroDef {
    pub num_params: usize,
    pub body: String,
    pub optional_param: Option<String>,
    /// TeX delimited parameter text (`\def\foo#1.#2\par{...}`): for each
    /// parameter, the literal text that must follow it (empty = undelimited).
    pub delimiters: Vec<String>,
    /// Literal text that must directly follow the macro name before the
    /// first parameter (`\def\foo(#1){...}` -> "(").
    pub prefix: String,
    /// xparse argument specification (`\NewDocumentCommand{\x}{s o m}`);
    /// when set it replaces `num_params`/`optional_param` for parsing.
    pub xparse: Option<Vec<XArg>>,
}

/// One xparse argument specifier.
#[derive(Debug, Clone, PartialEq)]
pub enum XArg {
    /// `m`: mandatory (braced group or single token)
    Mandatory,
    /// `o` / `O{default}`: optional `[...]`; None default means -NoValue-
    Optional(Option<String>),
    /// `s`: star; `t<c>`: test for token c
    Token(char),
    /// `d<>` / `D<>{default}` / `r<>`: delimited
    Delimited(char, char, Option<String>),
    /// `v`: verbatim group
    Verbatim,
}

#[derive(Debug, Clone)]
pub struct TheoremEnvDef {
    pub display_name: String,
    pub numbered: bool,
    pub counter_key: String,
    pub within: Option<String>,
}

/// User-defined environment from \newenvironment{name}[n]{begin}{end}
#[derive(Debug, Clone)]
pub struct EnvironmentDef {
    pub num_params: usize,
    pub optional_param: Option<String>,
    pub begin_def: String,
    pub end_def: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CitationStyle {
    Numeric,
    AuthorYear,
    Note,
}

impl ParserState {
    pub fn new() -> Self {
        Self {
            labels: HashMap::new(),
            footnote_counter: 0,
            footnote_marks: HashMap::new(),
            footnote_texts: HashMap::new(),
            figure_counter: 0,
            table_counter: 0,
            equation_counter: 0,
            section_counters: vec![0; 6], // 6 levels
            current_section_number: String::new(),
            current_level: 1,
            macros: HashMap::new(),
            theorem_envs: default_theorem_envs(),
            environments: HashMap::new(),
            theorem_counters: HashMap::new(),
            toggles: HashMap::new(),
            file_contents: HashMap::new(),
            included_files: Vec::new(),
            current_language: None,
            in_list_item: false,
            in_table_cell: false,
            metadata: HashMap::new(),
            bibliography: Vec::new(),
            citation_style: CitationStyle::Numeric,
            warnings: Vec::new(),
            appendix: false,
            has_chapters: false,
            numberwithin: HashMap::new(),
            conditionals: HashMap::new(),
            aliases: HashMap::new(),
        }
    }

    /// Enter appendix mode (`\appendix`): the top-level sectioning counter
    /// restarts and is rendered as a letter.
    pub fn start_appendix(&mut self) {
        self.appendix = true;
        let top = self.top_level();
        self.section_counters[top] = 0;
        for i in (top + 1)..self.section_counters.len() {
            self.section_counters[i] = 0;
        }
        self.update_section_number();
    }

    /// Index of the top-level numbered sectioning unit (0 = chapter, 1 = section).
    fn top_level(&self) -> usize {
        if self.has_chapters {
            0
        } else {
            1
        }
    }

    /// Set a counter from `\setcounter{name}{value}` (value is the number of
    /// the *last* item, as in LaTeX).
    pub fn set_counter(&mut self, name: &str, value: usize) {
        match name {
            "chapter" => self.section_counters[0] = value,
            "section" => self.section_counters[1] = value,
            "subsection" => self.section_counters[2] = value,
            "subsubsection" => self.section_counters[3] = value,
            "paragraph" => self.section_counters[4] = value,
            "subparagraph" => self.section_counters[5] = value,
            "equation" => self.equation_counter = value,
            "figure" => self.figure_counter = value,
            "table" => self.table_counter = value,
            "footnote" => self.footnote_counter = value,
            _ => return,
        }
        self.update_section_number();
    }

    /// Parent sectioning level a float/equation counter is numbered within.
    fn counter_parent_level(&self, counter: &str) -> Option<usize> {
        if let Some(parent) = self.numberwithin.get(counter) {
            return level_of_sectioning(parent);
        }
        if self.has_chapters && matches!(counter, "equation" | "figure" | "table") {
            Some(0)
        } else {
            None
        }
    }

    /// Step a float/equation counter and return its formatted number
    /// (e.g. `3`, or `2.3` when numbered within chapters).
    pub fn next_counter(&mut self, counter: &str) -> String {
        let value = match counter {
            "equation" => {
                self.equation_counter += 1;
                self.equation_counter
            }
            "figure" => {
                self.figure_counter += 1;
                self.figure_counter
            }
            "table" => {
                self.table_counter += 1;
                self.table_counter
            }
            _ => return String::new(),
        };
        match self.counter_parent_level(counter) {
            Some(level) => {
                let prefix = self.section_number_up_to(level);
                if prefix.is_empty() {
                    value.to_string()
                } else {
                    format!("{}.{}", prefix, value)
                }
            }
            None => value.to_string(),
        }
    }

    /// Formatted section number including levels 0..=max_level.
    pub fn section_number_up_to(&self, max_level: usize) -> String {
        let top = self.top_level();
        let mut parts = Vec::new();
        for (level, &count) in self.section_counters.iter().enumerate().take(max_level + 1) {
            if level < top && count == 0 {
                continue;
            }
            parts.push(self.format_level(level, count));
        }
        // Trailing zero levels (e.g. "2.0") are dropped
        while parts.len() > 1 && parts.last().map(String::as_str) == Some("0") {
            parts.pop();
        }
        parts.join(".")
    }

    fn format_level(&self, level: usize, count: usize) -> String {
        if self.appendix && level == self.top_level() && count > 0 {
            alpha_number(count, true)
        } else {
            count.to_string()
        }
    }

    pub fn warn(&mut self, message: String) {
        self.warnings.push(message);
    }

    pub fn increment_section(&mut self, level: usize) {
        if level < self.section_counters.len() {
            if level == 0 {
                self.has_chapters = true;
            }
            self.section_counters[level] += 1;

            // Reset lower levels
            for i in (level + 1)..self.section_counters.len() {
                self.section_counters[i] = 0;
            }

            // Reset counters numbered within this level
            for counter in ["equation", "figure", "table"] {
                if let Some(parent) = self.counter_parent_level(counter) {
                    if parent >= level {
                        match counter {
                            "equation" => self.equation_counter = 0,
                            "figure" => self.figure_counter = 0,
                            _ => self.table_counter = 0,
                        }
                    }
                }
            }

            self.current_level = level;
            self.update_section_number();
        }
    }

    fn update_section_number(&mut self) {
        let level = self.current_level;
        self.current_section_number = self.section_number_up_to(level);
    }

    pub fn add_label(&mut self, label: String, label_type: LabelType, title: Option<String>) {
        let number = match label_type {
            LabelType::Section => self.current_section_number.clone(),
            LabelType::Figure => self.next_counter("figure"),
            LabelType::Table => self.next_counter("table"),
            LabelType::Equation => self.next_counter("equation"),
            _ => String::new(),
        };

        self.labels.insert(
            label,
            LabelInfo {
                label_type,
                number,
                title,
            },
        );
    }

    pub fn add_label_with_number(
        &mut self,
        label: String,
        label_type: LabelType,
        number: String,
        title: Option<String>,
    ) {
        self.labels.insert(
            label,
            LabelInfo {
                label_type,
                number,
                title,
            },
        );
    }

    pub fn get_label(&self, label: &str) -> Option<&LabelInfo> {
        self.labels.get(label)
    }

    pub fn define_macro(&mut self, name: String, num_params: usize, body: String) {
        self.macros.insert(
            name,
            MacroDef {
                num_params,
                body,
                optional_param: None,
                ..Default::default()
            },
        );
    }

    pub fn expand_macro(&self, name: &str, args: &[String]) -> Option<String> {
        self.macros
            .get(name)
            .map(|def| substitute_params(&def.body, args))
    }

    pub fn define_theorem_env(
        &mut self,
        env_name: String,
        display_name: String,
        numbered: bool,
        counter_key: Option<String>,
        within: Option<String>,
    ) {
        let counter_key = counter_key.unwrap_or_else(|| env_name.clone());
        self.theorem_envs.insert(
            env_name,
            TheoremEnvDef {
                display_name,
                numbered,
                counter_key,
                within,
            },
        );
    }

    pub fn get_theorem_env(&self, env_name: &str) -> Option<&TheoremEnvDef> {
        self.theorem_envs.get(env_name)
    }

    pub fn next_theorem_number(&mut self, env_name: &str) -> Option<String> {
        let def = self.theorem_envs.get(env_name)?.clone();
        if !def.numbered {
            return None;
        }

        let scope_prefix = def
            .within
            .as_ref()
            .map(|within| self.section_number_for_counter(within));
        let scoped_key = if let Some(scope) = &scope_prefix {
            format!("{}@{}", def.counter_key, scope)
        } else {
            def.counter_key.clone()
        };

        let counter = self.theorem_counters.entry(scoped_key).or_insert(0);
        *counter += 1;
        if let Some(prefix) = scope_prefix {
            if prefix.is_empty() {
                Some(counter.to_string())
            } else {
                Some(format!("{}.{}", prefix, counter))
            }
        } else {
            Some(counter.to_string())
        }
    }

    fn section_number_for_counter(&self, name: &str) -> String {
        let max_level = match name {
            "chapter" => Some(0),
            "section" => Some(1),
            "subsection" => Some(2),
            "subsubsection" => Some(3),
            "paragraph" => Some(4),
            "subparagraph" => Some(5),
            _ => None,
        };

        let Some(max_level) = max_level else {
            return self.current_section_number.clone();
        };

        self.section_number_up_to(max_level)
    }

    pub fn set_toggle(&mut self, name: String, value: bool) {
        self.toggles.insert(name, value);
    }

    pub fn get_toggle(&self, name: &str) -> bool {
        self.toggles.get(name).copied().unwrap_or(false)
    }

    pub fn add_footnote_mark(&mut self, mark: String) -> usize {
        self.footnote_counter += 1;
        self.footnote_marks.insert(self.footnote_counter, mark);
        self.footnote_counter
    }

    pub fn add_footnote_text(&mut self, num: usize, text: String) {
        self.footnote_texts.insert(num, text);
    }

    pub fn resolve_footnotes(&self) -> HashMap<usize, (String, String)> {
        let mut resolved = HashMap::new();
        for (&num, mark) in &self.footnote_marks {
            if let Some(text) = self.footnote_texts.get(&num) {
                resolved.insert(num, (mark.clone(), text.clone()));
            }
        }
        resolved
    }

    pub fn can_include_file(&self, path: &str) -> bool {
        !self.included_files.contains(&path.to_string())
    }

    pub fn mark_file_included(&mut self, path: String) {
        self.included_files.push(path);
    }

    /// The file is no longer being read (its content was fully consumed):
    /// it may be included again, as long as it is not open recursively.
    pub fn unmark_file_included(&mut self, path: &str) {
        if let Some(pos) = self.included_files.iter().rposition(|p| p == path) {
            self.included_files.remove(pos);
        }
    }
}

/// Substitute `#1`..`#9` in a macro body in a single pass (so argument text
/// containing `#2` is never substituted again); `##` becomes `#`, as when a
/// macro body defines another macro.
pub fn substitute_params(body: &str, args: &[String]) -> String {
    let mut out = String::with_capacity(body.len());
    let mut chars = body.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '#' {
            match chars.peek().copied() {
                Some('#') => {
                    chars.next();
                    out.push('#');
                }
                Some(d @ '1'..='9') => {
                    chars.next();
                    let index = d as usize - '1' as usize;
                    if let Some(arg) = args.get(index) {
                        // `\foo#1` with #1 = "x" must not become `\foox`:
                        // in TeX they are separate tokens, so separate them
                        // with a (tokenizer-ignored) space
                        if arg.starts_with(|c: char| c.is_alphabetic() || c == '@')
                            && ends_with_control_word(&out)
                        {
                            out.push(' ');
                        }
                        out.push_str(arg);
                    }
                }
                _ => out.push('#'),
            }
        } else {
            out.push(ch);
        }
    }
    out
}

/// Does the text end with a control word (`\\name`)?
fn ends_with_control_word(text: &str) -> bool {
    let trimmed = text.trim_end_matches(|c: char| c.is_alphabetic() || c == '@');
    trimmed.len() < text.len() && trimmed.ends_with('\\') && {
        // the backslash itself must not be escaped (`\\\\foo` is a line break + foo)
        let backslashes = trimmed.chars().rev().take_while(|&c| c == '\\').count();
        backslashes % 2 == 1
    }
}

/// Sectioning level of a counter name (`chapter` = 0, `section` = 1, ...).
pub fn level_of_sectioning(name: &str) -> Option<usize> {
    match name.trim() {
        "part" | "chapter" => Some(0),
        "section" => Some(1),
        "subsection" => Some(2),
        "subsubsection" => Some(3),
        "paragraph" => Some(4),
        "subparagraph" => Some(5),
        _ => None,
    }
}

/// Render a counter value as a letter sequence: 1 -> a/A, 26 -> z/Z, 27 -> aa/AA.
pub fn alpha_number(mut n: usize, upper: bool) -> String {
    let mut out = Vec::new();
    while n > 0 {
        n -= 1;
        let base = if upper { b'A' } else { b'a' };
        out.push((base + (n % 26) as u8) as char);
        n /= 26;
    }
    out.iter().rev().collect()
}

/// Render a counter value as a Roman numeral.
pub fn roman_number(mut n: usize, upper: bool) -> String {
    if n == 0 || n >= 4000 {
        return n.to_string();
    }
    let table = [
        (1000, "m"), (900, "cm"), (500, "d"), (400, "cd"), (100, "c"), (90, "xc"),
        (50, "l"), (40, "xl"), (10, "x"), (9, "ix"), (5, "v"), (4, "iv"), (1, "i"),
    ];
    let mut out = String::new();
    for (value, numeral) in table {
        while n >= value {
            out.push_str(numeral);
            n -= value;
        }
    }
    if upper {
        out.to_uppercase()
    } else {
        out
    }
}

impl Default for ParserState {
    fn default() -> Self {
        Self::new()
    }
}

fn default_theorem_envs() -> HashMap<String, TheoremEnvDef> {
    let mut envs = HashMap::new();
    for (env_name, display_name, numbered, counter_key) in [
        ("theorem", "Theorem", true, "theorem"),
        ("lemma", "Lemma", true, "lemma"),
        ("proposition", "Proposition", true, "proposition"),
        ("corollary", "Corollary", true, "corollary"),
        ("definition", "Definition", true, "definition"),
        ("example", "Example", true, "example"),
        ("proof", "Proof", false, "proof"),
    ] {
        envs.insert(
            env_name.to_string(),
            TheoremEnvDef {
                display_name: display_name.to_string(),
                numbered,
                counter_key: counter_key.to_string(),
                within: None,
            },
        );
    }
    envs
}
