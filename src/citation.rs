use crate::error::Result;
use crate::state::CitationStyle;
use std::collections::HashMap;

/// Citation and bibliography management system
#[derive(Clone)]
pub struct CitationManager {
    /// Citation style
    style: CitationStyle,
    
    /// Bibliography database (key -> entry)
    bibliography: HashMap<String, BibEntry>,
    
    /// Citation counter for numeric style
    citation_counter: usize,
    
    /// Citation key to number mapping
    citation_numbers: HashMap<String, usize>,
}

#[derive(Debug, Clone)]
pub struct BibEntry {
    pub entry_type: String,
    pub key: String,
    pub fields: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CitationMode {
    Normal,      // \cite
    Textual,     // \citet
    Parenthetical, // \citep
    Author,      // \citeauthor
    Year,        // \citeyear
    YearPar,     // \citeyearpar
    Alt,         // \citealt
    Alp,         // \citealp
}

impl CitationManager {
    pub fn new(style: CitationStyle) -> Self {
        Self {
            style,
            bibliography: HashMap::new(),
            citation_counter: 0,
            citation_numbers: HashMap::new(),
        }
    }

    /// Set citation style
    pub fn set_style(&mut self, style: CitationStyle) {
        self.style = style;
    }

    /// Add a bibliography entry
    pub fn add_entry(&mut self, entry: BibEntry) {
        self.bibliography.insert(entry.key.clone(), entry);
    }

    pub fn has_entries(&self) -> bool {
        !self.bibliography.is_empty()
    }

    pub fn has_entry(&self, key: &str) -> bool {
        self.bibliography.contains_key(key)
    }

    /// Parse a .bib file content
    pub fn parse_bib_file(&mut self, content: &str) -> Result<()> {
        let entries = parse_bibtex(content)?;
        for entry in entries {
            self.add_entry(entry);
        }
        Ok(())
    }

    /// Format a citation
    pub fn format_citation(
        &mut self,
        keys: &[String],
        mode: CitationMode,
        prenote: Option<&str>,
        postnote: Option<&str>,
    ) -> String {
        match self.style {
            CitationStyle::Numeric => self.format_numeric(keys, mode, prenote, postnote),
            CitationStyle::AuthorYear => self.format_author_year(keys, mode, prenote, postnote),
            CitationStyle::Note => self.format_note(keys, mode, prenote, postnote),
        }
    }

    fn format_numeric(
        &mut self,
        keys: &[String],
        mode: CitationMode,
        prenote: Option<&str>,
        postnote: Option<&str>,
    ) -> String {
        let numbers: Vec<String> = keys
            .iter()
            .map(|key| {
                let num = self.citation_numbers.entry(key.clone()).or_insert_with(|| {
                    self.citation_counter += 1;
                    self.citation_counter
                });
                num.to_string()
            })
            .collect();

        let citation_text = numbers.join(", ");

        match mode {
            CitationMode::Normal | CitationMode::Parenthetical => {
                let mut result = String::from("[");
                if let Some(pre) = prenote {
                    result.push_str(pre);
                    result.push_str(", ");
                }
                result.push_str(&citation_text);
                if let Some(post) = postnote {
                    result.push_str(", ");
                    result.push_str(post);
                }
                result.push(']');
                result
            }
            CitationMode::Textual => {
                format!("{}[{}]", self.get_author_text(keys), citation_text)
            }
            CitationMode::Author => self.get_author_text(keys),
            CitationMode::Year | CitationMode::YearPar => {
                format!("[{}]", citation_text)
            }
            CitationMode::Alt | CitationMode::Alp => citation_text,
        }
    }

    fn format_author_year(
        &mut self,
        keys: &[String],
        mode: CitationMode,
        prenote: Option<&str>,
        postnote: Option<&str>,
    ) -> String {
        let citations: Vec<String> = keys
            .iter()
            .map(|key| {
                if let Some(entry) = self.bibliography.get(key) {
                    let author = entry
                        .fields
                        .get("author")
                        .map(|names| short_author_names(names))
                        .unwrap_or_else(|| key.clone());
                    let year = entry.fields.get("year").map(|s| s.as_str()).unwrap_or("n.d.");

                    match mode {
                        CitationMode::Author => author,
                        CitationMode::Year => year.to_string(),
                        CitationMode::YearPar => format!("({})", year),
                        CitationMode::Textual => format!("{} ({})", author, year),
                        CitationMode::Alt => format!("{} {}", author, year),
                        _ => format!("{}, {}", author, year),
                    }
                } else {
                    key.clone()
                }
            })
            .collect();

        let citation_text = citations.join("; ");

        match mode {
            CitationMode::Normal | CitationMode::Parenthetical => {
                let mut result = String::from("(");
                if let Some(pre) = prenote {
                    result.push_str(pre);
                    result.push(' ');
                }
                result.push_str(&citation_text);
                if let Some(post) = postnote {
                    result.push_str(", ");
                    result.push_str(post);
                }
                result.push(')');
                result
            }
            CitationMode::Textual | CitationMode::Alt | CitationMode::Alp => citation_text,
            CitationMode::Author | CitationMode::Year | CitationMode::YearPar => citation_text,
        }
    }

    fn format_note(
        &mut self,
        keys: &[String],
        _mode: CitationMode,
        prenote: Option<&str>,
        postnote: Option<&str>,
    ) -> String {
        let citations: Vec<String> = keys
            .iter()
            .map(|key| {
                if let Some(entry) = self.bibliography.get(key) {
                    self.format_full_citation(entry)
                } else {
                    key.clone()
                }
            })
            .collect();

        let mut result = String::new();
        if let Some(pre) = prenote {
            result.push_str(pre);
            result.push_str(". ");
        }
        result.push_str(&citations.join("; "));
        if let Some(post) = postnote {
            result.push_str(", ");
            result.push_str(post);
        }
        result
    }

    fn get_author_text(&self, keys: &[String]) -> String {
        keys.iter()
            .filter_map(|key| {
                self.bibliography
                    .get(key)
                    .and_then(|entry| entry.fields.get("author"))
                    .map(|s| s.as_str())
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn format_full_citation(&self, entry: &BibEntry) -> String {
        let author = entry
            .fields
            .get("author")
            .map(|names| format_author_list(names))
            .unwrap_or_default();
        let author = author.as_str();
        let title = entry.fields.get("title").map(|s| s.as_str()).unwrap_or("");
        let year = entry.fields.get("year").map(|s| s.as_str()).unwrap_or("");

        match entry.entry_type.as_str() {
            "article" => {
                let journal = entry.fields.get("journal").map(|s| s.as_str()).unwrap_or("");
                format!("{}. \"{}\". *{}* ({})", author, title, journal, year)
            }
            "book" => {
                let publisher = entry.fields.get("publisher").map(|s| s.as_str()).unwrap_or("");
                format!("{}. *{}*. {} ({})", author, title, publisher, year)
            }
            _ => format!("{}. \"{}\" ({})", author, title, year),
        }
    }

    /// Generate bibliography in Markdown format
    pub fn generate_bibliography(&self) -> String {
        let mut result = String::from("## References\n\n");

        match self.style {
            CitationStyle::Numeric => {
                // Only cited entries appear, in citation-number order
                let mut cited: Vec<_> = self
                    .bibliography
                    .values()
                    .filter_map(|entry| {
                        self.citation_numbers
                            .get(&entry.key)
                            .map(|&num| (num, entry))
                    })
                    .collect();
                cited.sort_by_key(|(num, _)| *num);
                for (num, entry) in cited {
                    result.push_str(&format!(
                        "[{}] {}\n\n",
                        num,
                        self.format_full_citation(entry)
                    ));
                }
            }
            CitationStyle::AuthorYear | CitationStyle::Note => {
                let mut entries: Vec<_> = self.bibliography.values().collect();
                entries.sort_by_key(|entry| {
                    (
                        entry
                            .fields
                            .get("author")
                            .map(|author| surname(author.split(" and ").next().unwrap_or(author)))
                            .unwrap_or_else(|| entry.key.clone()),
                        entry.fields.get("year").cloned().unwrap_or_default(),
                    )
                });
                for entry in entries {
                    result.push_str(&format!("- {}\n\n", self.format_full_citation(entry)));
                }
            }
        }

        result
    }
}

/// Render a BibTeX author field ("A and B and C") as a readable list
/// ("A, B, and C"), normalizing "Last, First" to "First Last".
fn format_author_list(field: &str) -> String {
    let authors: Vec<String> = field
        .split(" and ")
        .map(|name| {
            let name = name.trim();
            if let Some((last, first)) = name.split_once(',') {
                format!("{} {}", first.trim(), last.trim())
            } else {
                name.to_string()
            }
        })
        .collect();
    match authors.len() {
        0 => String::new(),
        1 => authors[0].clone(),
        2 => format!("{} and {}", authors[0], authors[1]),
        _ => format!(
            "{}, and {}",
            authors[..authors.len() - 1].join(", "),
            authors[authors.len() - 1]
        ),
    }
}

/// Reduce a BibTeX author field to citation-style surnames:
/// one author "Knuth", two "Knuth and Lamport", three+ "Knuth et al."
fn short_author_names(field: &str) -> String {
    let authors: Vec<&str> = field.split(" and ").map(str::trim).collect();
    let surnames: Vec<String> = authors.iter().map(|name| surname(name)).collect();
    match surnames.len() {
        0 => String::new(),
        1 => surnames[0].clone(),
        2 => format!("{} and {}", surnames[0], surnames[1]),
        _ => format!("{} et al.", surnames[0]),
    }
}

/// Extract the surname from "First Last" or "Last, First" BibTeX name forms.
fn surname(name: &str) -> String {
    if let Some((last, _)) = name.split_once(',') {
        last.trim().to_string()
    } else {
        name.split_whitespace()
            .last()
            .unwrap_or(name)
            .to_string()
    }
}

/// Parse BibTeX content. Character-level parser: handles single-line and
/// multi-line entries, nested braces in values ({The {LaTeX} Companion}),
/// quoted and bare values, and @comment/@preamble/@string blocks.
fn parse_bibtex(content: &str) -> Result<Vec<BibEntry>> {
    let chars: Vec<char> = content.chars().collect();
    let mut entries = Vec::new();
    let mut i = 0;

    while i < chars.len() {
        if chars[i] != '@' {
            i += 1;
            continue;
        }
        i += 1;

        // Entry type
        let start = i;
        while i < chars.len() && chars[i].is_alphabetic() {
            i += 1;
        }
        let entry_type: String = chars[start..i].iter().collect::<String>().to_lowercase();

        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        if i >= chars.len() || (chars[i] != '{' && chars[i] != '(') {
            continue;
        }
        let close = if chars[i] == '{' { '}' } else { ')' };
        i += 1;

        // Non-entry blocks: skip to the matching close delimiter
        if matches!(entry_type.as_str(), "comment" | "preamble" | "string") {
            let mut depth = 1;
            while i < chars.len() && depth > 0 {
                match chars[i] {
                    '{' => depth += 1,
                    '}' if close == '}' => depth -= 1,
                    ')' if close == ')' => depth -= 1,
                    _ => {}
                }
                i += 1;
            }
            continue;
        }

        // Citation key (up to first comma)
        let start = i;
        while i < chars.len() && chars[i] != ',' && chars[i] != close {
            i += 1;
        }
        let key = chars[start..i].iter().collect::<String>().trim().to_string();

        let mut fields = HashMap::new();

        // Fields: name = value pairs separated by commas
        while i < chars.len() && chars[i] != close {
            if chars[i] == ',' || chars[i].is_whitespace() {
                i += 1;
                continue;
            }

            let start = i;
            while i < chars.len() && chars[i] != '=' && chars[i] != close {
                i += 1;
            }
            if i >= chars.len() || chars[i] == close {
                break;
            }
            let name: String = chars[start..i]
                .iter()
                .collect::<String>()
                .trim()
                .to_lowercase();
            i += 1; // consume '='
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }

            let mut value = String::new();
            if i < chars.len() && chars[i] == '{' {
                let mut depth = 1;
                i += 1;
                while i < chars.len() && depth > 0 {
                    match chars[i] {
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                i += 1;
                                break;
                            }
                        }
                        ch => value.push(ch),
                    }
                    i += 1;
                }
            } else if i < chars.len() && chars[i] == '"' {
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    value.push(chars[i]);
                    i += 1;
                }
                if i < chars.len() {
                    i += 1;
                }
            } else {
                while i < chars.len() && chars[i] != ',' && chars[i] != close {
                    value.push(chars[i]);
                    i += 1;
                }
            }

            if !name.is_empty() {
                fields.insert(name, normalize_bib_value(&value));
            }
        }
        if i < chars.len() {
            i += 1; // consume the closing delimiter
        }

        if !key.is_empty() {
            entries.push(BibEntry {
                entry_type,
                key,
                fields,
            });
        }
    }

    Ok(entries)
}

/// Clean a raw BibTeX field value for Markdown output: collapse whitespace,
/// resolve TeX ties/dashes/escapes.
fn normalize_bib_value(value: &str) -> String {
    let collapsed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed
        .replace('~', "\u{00A0}")
        .replace("---", "\u{2014}")
        .replace("--", "\u{2013}")
        .replace("\\&", "&")
        .replace("\\%", "%")
        .replace("\\_", "_")
        .replace("\\$", "$")
        .replace("\\#", "#")
}

/// Parse citation command arguments
pub fn parse_citation_args(input: &str) -> (Vec<String>, Option<String>, Option<String>) {
    let mut keys = Vec::new();
    let mut prenote = None;
    let mut postnote = None;

    let mut chars = input.chars().peekable();

    // Skip whitespace
    while chars.peek() == Some(&' ') || chars.peek() == Some(&'\t') {
        chars.next();
    }

    // Parse optional prenote [prenote]
    if chars.peek() == Some(&'[') {
        chars.next();
        let note: String = chars.by_ref().take_while(|&c| c != ']').collect();
        prenote = Some(note);
        
        // Skip whitespace
        while chars.peek() == Some(&' ') || chars.peek() == Some(&'\t') {
            chars.next();
        }

        // Parse optional postnote [postnote]
        if chars.peek() == Some(&'[') {
            chars.next();
            let note: String = chars.by_ref().take_while(|&c| c != ']').collect();
            postnote = Some(note);
        }
    }

    // Skip whitespace
    while chars.peek() == Some(&' ') || chars.peek() == Some(&'\t') {
        chars.next();
    }

    // Parse citation keys {key1,key2,...}
    if chars.peek() == Some(&'{') {
        chars.next();
        let keys_str: String = chars.by_ref().take_while(|&c| c != '}').collect();
        keys = keys_str.split(',').map(|s| s.trim().to_string()).collect();
    }

    (keys, prenote, postnote)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_citation_args() {
        let (keys, prenote, postnote) = parse_citation_args("{key1,key2}");
        assert_eq!(keys, vec!["key1", "key2"]);
        assert_eq!(prenote, None);
        assert_eq!(postnote, None);
    }

    #[test]
    fn test_parse_citation_args_with_notes() {
        let (keys, prenote, postnote) = parse_citation_args("[see][p. 10]{key1}");
        assert_eq!(keys, vec!["key1"]);
        assert_eq!(prenote, Some("see".to_string()));
        assert_eq!(postnote, Some("p. 10".to_string()));
    }

    #[test]
    fn test_numeric_citation() {
        let mut manager = CitationManager::new(CitationStyle::Numeric);
        let result = manager.format_citation(
            &["key1".to_string()],
            CitationMode::Normal,
            None,
            None,
        );
        assert_eq!(result, "[1]");
    }

    #[test]
    fn test_parse_bibtex() {
        let bib = r#"
@article{key1,
  author = {John Doe},
  title = {A Great Paper},
  year = {2023},
  journal = {Nature}
}
"#;
        let entries = parse_bibtex(bib).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].key, "key1");
        assert_eq!(entries[0].entry_type, "article");
    }
}
