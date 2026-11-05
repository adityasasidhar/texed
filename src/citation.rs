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

    /// Keys cited so far (\cite, \nocite): only these are listed, as BibTeX
    /// does, unless `\nocite{*}` asked for every entry
    cited: std::collections::HashSet<String>,
    cite_all: bool,
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
    Title,       // \citetitle (biblatex)
    Url,         // \citeurl (biblatex)
    Field(String), // \citefield{key}{field}, \citedate, \citeyear*
    Full,        // \fullcite
}

impl CitationManager {
    pub fn new(style: CitationStyle) -> Self {
        Self {
            style,
            bibliography: HashMap::new(),
            citation_counter: 0,
            citation_numbers: HashMap::new(),
            cited: std::collections::HashSet::new(),
            cite_all: false,
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
        self.cited.extend(keys.iter().cloned());
        match &mode {
            CitationMode::Title => return self.entry_fields(keys, "title"),
            CitationMode::Url => return self.entry_fields(keys, "url"),
            CitationMode::Field(field) => return self.entry_fields(keys, &field.clone()),
            CitationMode::Full => {
                return keys
                    .iter()
                    .map(|key| match self.bibliography.get(key) {
                        Some(entry) => self.format_full_citation(entry),
                        None => escape_latex(key),
                    })
                    .collect::<Vec<_>>()
                    .join("; ")
            }
            _ => {}
        }
        match self.style {
            CitationStyle::Numeric => self.format_numeric(keys, mode, prenote, postnote),
            CitationStyle::AuthorYear => self.format_author_year(keys, mode, prenote, postnote),
            CitationStyle::Note => self.format_note(keys, mode, prenote, postnote),
        }
    }

    /// A bibliography field of each cited entry (`\citetitle`, `\citeurl`),
    /// the key itself when the entry or field is unknown.
    fn entry_fields(&self, keys: &[String], field: &str) -> String {
        keys.iter()
            .map(|key| {
                self.bibliography
                    .get(key)
                    .and_then(|entry| entry.fields.get(field))
                    .map(|value| match field {
                        "url" => format!("\\url{{{}}}", value),
                        _ => value.clone(),
                    })
                    .unwrap_or_else(|| escape_latex(key))
            })
            .collect::<Vec<_>>()
            .join("; ")
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
            _ => citation_text,
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
                        .unwrap_or_else(|| escape_latex(key));
                    let year = entry
                        .fields
                        .get("year")
                        .map(|s| s.as_str())
                        .or_else(|| entry.fields.get("date").map(|d| d.split(['-', '/']).next().unwrap_or(d)))
                        .unwrap_or("n.d.");

                    match mode {
                        CitationMode::Author => author,
                        CitationMode::Year => year.to_string(),
                        CitationMode::YearPar => format!("({})", year),
                        CitationMode::Textual => format!("{} ({})", author, year),
                        CitationMode::Alt => format!("{} {}", author, year),
                        _ => format!("{}, {}", author, year),
                    }
                } else {
                    escape_latex(key)
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
            _ => citation_text,
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
                    escape_latex(key)
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
                    .and_then(|entry| entry.fields.get("author").or_else(|| entry.fields.get("editor")))
                    .map(|names| short_author_names(names))
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// One full reference as LaTeX source (the parser renders it, so TeX
    /// accents, `\emph`, dashes and special characters come out right):
    /// `Author. \enquote{Title}. \emph{Journal} 12(3), pp. 1--9. Publisher, 2020. \url{..}`
    fn format_full_citation(&self, entry: &BibEntry) -> String {
        let field = |name: &str| entry.fields.get(name).map(|v| v.trim()).filter(|v| !v.is_empty());
        let mut parts: Vec<String> = Vec::new();
        if let Some(author) = field("author").or_else(|| field("editor")) {
            let mut names = format_author_list(author);
            if field("author").is_none() {
                names.push_str(if author.contains(" and ") { " (eds.)" } else { " (ed.)" });
            }
            parts.push(names);
        }
        let book_like = matches!(
            entry.entry_type.as_str(),
            "book" | "mvbook" | "booklet" | "manual" | "phdthesis" | "mastersthesis" | "thesis" | "proceedings" | "collection" | "report" | "techreport"
        );
        if let Some(title) = field("title") {
            parts.push(if book_like {
                format!("\\emph{{{}}}", title)
            } else {
                format!("\\enquote{{{}}}", title)
            });
        }
        let mut container = String::new();
        if let Some(journal) = field("journal").or_else(|| field("journaltitle")).or_else(|| field("booktitle")) {
            if field("booktitle").is_some() && field("journal").is_none() && field("journaltitle").is_none() {
                container.push_str("In ");
            }
            container.push_str(&format!("\\emph{{{}}}", journal));
        }
        if let Some(volume) = field("volume") {
            if !container.is_empty() {
                container.push(' ');
            }
            container.push_str(volume);
            if let Some(number) = field("number").or_else(|| field("issue")) {
                container.push_str(&format!("({})", number));
            }
        }
        if let Some(pages) = field("pages") {
            if !container.is_empty() {
                container.push_str(", ");
            }
            let prefix = if pages.contains('-') || pages.contains(',') { "pp." } else { "p." };
            container.push_str(&format!("{}~{}", prefix, pages));
        }
        if !container.is_empty() {
            parts.push(container);
        }
        let mut publication: Vec<&str> = Vec::new();
        for name in ["howpublished", "publisher", "organization", "institution", "school", "address"] {
            if let Some(value) = field(name) {
                if !value.starts_with("\\url") && !value.starts_with("http") {
                    publication.push(value);
                }
            }
        }
        let year = field("year").map(str::to_string).or_else(|| {
            field("date").map(|date| date.split(['-', '/']).next().unwrap_or(date).to_string())
        });
        let mut tail = publication.join(", ");
        if let Some(year) = year {
            if !tail.is_empty() {
                tail.push_str(", ");
            }
            tail.push_str(&year);
        }
        if !tail.is_empty() {
            parts.push(tail);
        }
        if let Some(note) = field("note").or_else(|| field("addendum")) {
            parts.push(note.to_string());
        }
        let url = field("url")
            .map(str::to_string)
            .or_else(|| field("howpublished").and_then(url_of))
            .or_else(|| field("doi").map(|doi| format!("https://doi.org/{}", doi.trim_start_matches("https://doi.org/"))));
        if let Some(url) = url {
            parts.push(format!("\\url{{{}}}", url));
        }
        if parts.is_empty() {
            return escape_latex(&entry.key);
        }
        // a part already ending a sentence (also inside `\emph{...}`) takes
        // no extra period
        let ends_sentence = |text: &str| text.trim_end_matches('}').ends_with(['.', '?', '!']);
        let mut out = String::new();
        for part in parts {
            if !out.is_empty() {
                out.push_str(if ends_sentence(&out) { " " } else { ". " });
            }
            out.push_str(&part);
        }
        if !ends_sentence(&out) && !out.ends_with('}') {
            out.push('.');
        }
        out
    }

    /// `\nocite{keys}` (`*` for all): listed in the references without a
    /// citation in the text; numbered after the cited ones.
    pub fn nocite(&mut self, keys: &[String]) {
        for key in keys {
            if key == "*" {
                self.cite_all = true;
                continue;
            }
            self.cited.insert(key.clone());
            if self.bibliography.contains_key(key) && !self.citation_numbers.contains_key(key) {
                self.citation_counter += 1;
                self.citation_numbers.insert(key.clone(), self.citation_counter);
            }
        }
        if self.cite_all {
            let mut keys: Vec<String> = self.bibliography.keys().cloned().collect();
            keys.sort();
            for key in keys {
                if !self.citation_numbers.contains_key(&key) {
                    self.citation_counter += 1;
                    self.citation_numbers.insert(key, self.citation_counter);
                }
            }
        }
    }

    /// The reference list: `(marker, LaTeX source)` per entry, in order.
    /// Numeric style lists the cited entries by number (`[1]`); author-year
    /// and note styles list every entry, sorted by author and year, with no
    /// marker.
    pub fn bibliography_items(&self) -> Vec<(Option<String>, String)> {
        match self.style {
            CitationStyle::Numeric => {
                // Only cited entries appear, in citation-number order
                let mut cited: Vec<_> = self
                    .bibliography
                    .values()
                    .filter_map(|entry| self.citation_numbers.get(&entry.key).map(|&num| (num, entry)))
                    .collect();
                cited.sort_by_key(|(num, _)| *num);
                cited
                    .into_iter()
                    .map(|(num, entry)| (Some(format!("[{}]", num)), self.format_full_citation(entry)))
                    .collect()
            }
            CitationStyle::AuthorYear | CitationStyle::Note => {
                let mut entries: Vec<_> = self
                    .bibliography
                    .values()
                    .filter(|entry| self.cite_all || self.cited.contains(&entry.key))
                    .collect();
                entries.sort_by_key(|entry| {
                    (
                        entry
                            .fields
                            .get("author")
                            .map(|author| surname(author.split(" and ").next().unwrap_or(author)))
                            .unwrap_or_else(|| entry.key.clone()),
                        entry.fields.get("year").cloned().unwrap_or_default(),
                        entry.key.clone(),
                    )
                });
                entries
                    .into_iter()
                    .map(|entry| (None, self.format_full_citation(entry)))
                    .collect()
            }
        }
    }
}

/// The URL in a `howpublished = {\url{...}}` field.
fn url_of(value: &str) -> Option<String> {
    let rest = value.trim().strip_prefix("\\url{")?;
    let end = rest.rfind('}')?;
    Some(rest[..end].to_string())
}

/// Escape text so that LaTeX reproduces it literally (citation keys).
pub fn escape_latex(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\textbackslash{}"),
            '{' | '}' | '$' | '&' | '#' | '_' | '%' => {
                out.push('\\');
                out.push(c);
            }
            '^' => out.push_str("\\^{}"),
            '~' => out.push_str("\\textasciitilde{}"),
            _ => out.push(c),
        }
    }
    out
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

/// Parse BibTeX content. Character-level parser: single-line and
/// multi-line entries, `{...}` and `"..."` values with nested braces (kept,
/// so `{The {LaTeX} Companion}` and `\hologo{LuaTeX}` survive), bare numbers,
/// `@string` abbreviations and month names, `#` concatenation, and
/// `@comment`/`@preamble` blocks. Field values are LaTeX source.
fn parse_bibtex(content: &str) -> Result<Vec<BibEntry>> {
    let chars: Vec<char> = content.chars().collect();
    let mut entries = Vec::new();
    let mut strings: HashMap<String, String> = HashMap::new();
    for (i, month) in [
        "January", "February", "March", "April", "May", "June", "July", "August", "September", "October",
        "November", "December",
    ]
    .iter()
    .enumerate()
    {
        strings.insert(month[..3].to_lowercase(), month.to_string());
        let _ = i;
    }
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

        if matches!(entry_type.as_str(), "comment" | "preamble") {
            skip_to_close(&chars, &mut i, close);
            continue;
        }
        if entry_type == "string" {
            // @string{name = value}
            let fields = parse_fields(&chars, &mut i, close, &strings);
            strings.extend(fields);
            if i < chars.len() && chars[i] == close {
                i += 1;
            }
            continue;
        }

        // Citation key (up to first comma)
        let start = i;
        while i < chars.len() && chars[i] != ',' && chars[i] != close && chars[i] != '\n' {
            i += 1;
        }
        let key = chars[start..i].iter().collect::<String>().trim().to_string();
        let fields = parse_fields(&chars, &mut i, close, &strings);
        if i < chars.len() && chars[i] == close {
            i += 1; // consume the closing delimiter
        }

        if !key.is_empty() && !key.contains(char::is_whitespace) {
            entries.push(BibEntry {
                entry_type,
                key,
                fields,
            });
        }
    }

    Ok(entries)
}

/// Skip to just past the delimiter closing the current block.
fn skip_to_close(chars: &[char], i: &mut usize, close: char) {
    let mut depth = 1;
    while *i < chars.len() && depth > 0 {
        match chars[*i] {
            '{' => depth += 1,
            '}' if close == '}' => depth -= 1,
            '}' => depth -= 1,
            ')' if close == ')' && depth == 1 => depth -= 1,
            _ => {}
        }
        *i += 1;
    }
}

/// `name = value, ...` up to (not past) the entry's closing delimiter.
fn parse_fields(
    chars: &[char],
    i: &mut usize,
    close: char,
    strings: &HashMap<String, String>,
) -> HashMap<String, String> {
    let mut fields = HashMap::new();
    while *i < chars.len() && chars[*i] != close {
        if chars[*i] == ',' || chars[*i].is_whitespace() {
            *i += 1;
            continue;
        }
        // a new entry starting means this one was not closed
        if chars[*i] == '@' {
            break;
        }
        let start = *i;
        while *i < chars.len() && chars[*i] != '=' && chars[*i] != close && chars[*i] != ',' {
            *i += 1;
        }
        if *i >= chars.len() || chars[*i] != '=' {
            continue;
        }
        let name: String = chars[start..*i].iter().collect::<String>().trim().to_lowercase();
        *i += 1; // consume '='
        let value = parse_value(chars, i, close, strings);
        if !name.is_empty() {
            fields.insert(name, value.split_whitespace().collect::<Vec<_>>().join(" "));
        }
    }
    fields
}

/// A field value: parts joined by `#`.
fn parse_value(chars: &[char], i: &mut usize, close: char, strings: &HashMap<String, String>) -> String {
    let mut value = String::new();
    loop {
        while *i < chars.len() && chars[*i].is_whitespace() {
            *i += 1;
        }
        if *i >= chars.len() {
            break;
        }
        match chars[*i] {
            '{' => {
                *i += 1;
                value.push_str(&balanced(chars, i, '}'));
            }
            '"' => {
                *i += 1;
                value.push_str(&balanced(chars, i, '"'));
            }
            _ => {
                let start = *i;
                while *i < chars.len()
                    && chars[*i] != ','
                    && chars[*i] != close
                    && chars[*i] != '#'
                    && !chars[*i].is_whitespace()
                {
                    *i += 1;
                }
                let word: String = chars[start..*i].iter().collect();
                match strings.get(&word.to_lowercase()) {
                    Some(expansion) => value.push_str(expansion),
                    None => value.push_str(&word),
                }
            }
        }
        while *i < chars.len() && chars[*i].is_whitespace() {
            *i += 1;
        }
        if *i < chars.len() && chars[*i] == '#' {
            *i += 1;
            continue;
        }
        break;
    }
    value
}

/// Text up to the unnested `end` delimiter (consumed); inner braces kept.
/// An unbalanced value stops at a blank line, so one broken entry cannot
/// swallow the rest of the file.
fn balanced(chars: &[char], i: &mut usize, end: char) -> String {
    let mut depth = 0usize;
    let mut out = String::new();
    while *i < chars.len() {
        let c = chars[*i];
        if c == '\\' && *i + 1 < chars.len() {
            out.push(c);
            out.push(chars[*i + 1]);
            *i += 2;
            continue;
        }
        if depth == 0 && c == end {
            *i += 1;
            return out;
        }
        // a new entry at the start of a line: this value was never closed
        if c == '@' && out.trim_end_matches([' ', '\t']).ends_with('\n') {
            break;
        }
        match c {
            '{' => depth += 1,
            '}' if depth > 0 => depth -= 1,
            '}' => {
                // stray close brace ends the entry
                break;
            }
            _ => {}
        }
        out.push(c);
        *i += 1;
    }
    // unbalanced: close what was opened so the value is valid LaTeX
    out.push_str(&"}".repeat(depth));
    out
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
