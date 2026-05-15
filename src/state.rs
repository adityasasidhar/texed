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
    
    /// Macro definitions (name -> (params, body))
    pub macros: HashMap<String, MacroDef>,

    /// Theorem-like environment definitions
    pub theorem_envs: HashMap<String, TheoremEnvDef>,

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

#[derive(Debug, Clone)]
pub struct MacroDef {
    pub num_params: usize,
    pub body: String,
    pub optional_param: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TheoremEnvDef {
    pub display_name: String,
    pub numbered: bool,
    pub counter_key: String,
    pub within: Option<String>,
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
            macros: HashMap::new(),
            theorem_envs: default_theorem_envs(),
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
        }
    }

    pub fn increment_section(&mut self, level: usize) {
        if level < self.section_counters.len() {
            self.section_counters[level] += 1;
            
            // Reset lower levels
            for i in (level + 1)..self.section_counters.len() {
                self.section_counters[i] = 0;
            }
            
            // Update current section number
            self.update_section_number();
        }
    }

    fn update_section_number(&mut self) {
        let mut parts = Vec::new();
        let mut started = false;
        for &count in &self.section_counters {
            if count > 0 {
                started = true;
                parts.push(count.to_string());
            } else if started {
                break;
            }
        }
        self.current_section_number = parts.join(".");
    }

    pub fn add_label(&mut self, label: String, label_type: LabelType, title: Option<String>) {
        let number = match label_type {
            LabelType::Section => self.current_section_number.clone(),
            LabelType::Figure => {
                self.figure_counter += 1;
                self.figure_counter.to_string()
            }
            LabelType::Table => {
                self.table_counter += 1;
                self.table_counter.to_string()
            }
            LabelType::Equation => {
                self.equation_counter += 1;
                self.equation_counter.to_string()
            }
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
            },
        );
    }

    pub fn expand_macro(&self, name: &str, args: &[String]) -> Option<String> {
        self.macros.get(name).map(|def| {
            let mut result = def.body.clone();
            for (i, arg) in args.iter().enumerate() {
                let placeholder = format!("#{}", i + 1);
                result = result.replace(&placeholder, arg);
            }
            result
        })
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

        let mut parts = Vec::new();
        for count in self.section_counters.iter().take(max_level + 1) {
            if *count > 0 {
                parts.push(count.to_string());
            }
        }
        parts.join(".")
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
