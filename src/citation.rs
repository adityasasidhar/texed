use crate::error::Result;
use crate::state::{CitationStyle, ParserState};
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
                    let author = entry.fields.get("author").map(|s| s.as_str()).unwrap_or(key);
                    let year = entry.fields.get("year").map(|s| s.as_str()).unwrap_or("n.d.");
                    
                    match mode {
                        CitationMode::Author => author.to_string(),
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
                    result.push_str("; ");
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
        let author = entry.fields.get("author").map(|s| s.as_str()).unwrap_or("");
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
        let mut entries: Vec<_> = self.bibliography.values().collect();
        entries.sort_by_key(|e| &e.key);

        let mut result = String::from("## References\n\n");

        match self.style {
            CitationStyle::Numeric => {
                for entry in entries {
                    if let Some(&num) = self.citation_numbers.get(&entry.key) {
                        result.push_str(&format!("[{}] {}\n\n", num, self.format_full_citation(entry)));
                    }
                }
            }
            CitationStyle::AuthorYear | CitationStyle::Note => {
                for entry in entries {
                    result.push_str(&format!("- {}\n\n", self.format_full_citation(entry)));
                }
            }
        }

        result
    }
}

/// Parse BibTeX content
fn parse_bibtex(content: &str) -> Result<Vec<BibEntry>> {
    let mut entries = Vec::new();
    let mut current_entry: Option<BibEntry> = None;
    let mut in_entry = false;
    let mut brace_depth = 0;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with('@') {
            // Start of new entry
            if let Some(entry) = current_entry.take() {
                entries.push(entry);
            }

            let parts: Vec<&str> = trimmed[1..].splitn(2, '{').collect();
            if parts.len() == 2 {
                let entry_type = parts[0].trim().to_lowercase();
                let key = parts[1].trim_end_matches(',').trim().to_string();

                current_entry = Some(BibEntry {
                    entry_type,
                    key,
                    fields: HashMap::new(),
                });
                in_entry = true;
                brace_depth = 1;
            }
        } else if in_entry {
            // Count braces
            for ch in trimmed.chars() {
                match ch {
                    '{' => brace_depth += 1,
                    '}' => {
                        brace_depth -= 1;
                        if brace_depth == 0 {
                            in_entry = false;
                            if let Some(entry) = current_entry.take() {
                                entries.push(entry);
                            }
                            break;
                        }
                    }
                    _ => {}
                }
            }

            // Parse field
            if in_entry && trimmed.contains('=') {
                let parts: Vec<&str> = trimmed.splitn(2, '=').collect();
                if parts.len() == 2 {
                    let field_name = parts[0].trim().to_lowercase();
                    let field_value = parts[1]
                        .trim()
                        .trim_end_matches(',')
                        .trim_matches('{')
                        .trim_matches('}')
                        .trim_matches('"')
                        .to_string();

                    if let Some(ref mut entry) = current_entry {
                        entry.fields.insert(field_name, field_value);
                    }
                }
            }
        }
    }

    // Add last entry if exists
    if let Some(entry) = current_entry {
        entries.push(entry);
    }

    Ok(entries)
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
