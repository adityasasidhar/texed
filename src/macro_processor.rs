use crate::error::{Result, TexedError};
use crate::state::{MacroDef, ParserState};
use std::collections::HashMap;

/// Macro processor for LaTeX macro expansion
#[derive(Clone)]
pub struct MacroProcessor {
    /// Built-in macro definitions
    builtins: HashMap<String, MacroDef>,
}

impl MacroProcessor {
    pub fn new() -> Self {
        let mut processor = Self {
            builtins: HashMap::new(),
        };
        processor.register_builtins();
        processor
    }

    fn register_builtins(&mut self) {
        // Common LaTeX macros
        self.define_builtin("LaTeX", 0, "LaTeX");
        self.define_builtin("TeX", 0, "TeX");
        self.define_builtin("newline", 0, "\n");
        self.define_builtin("noindent", 0, "");
        self.define_builtin("par", 0, "\n\n");
        
        // Spacing
        self.define_builtin("smallskip", 0, "\n");
        self.define_builtin("medskip", 0, "\n\n");
        self.define_builtin("bigskip", 0, "\n\n\n");
        
        // Common abbreviations
        self.define_builtin("ie", 0, "i.e.");
        self.define_builtin("eg", 0, "e.g.");
        self.define_builtin("cf", 0, "cf.");
        self.define_builtin("etc", 0, "etc.");
        self.define_builtin("etal", 0, "et al.");
        self.define_builtin("vs", 0, "vs.");
    }

    fn define_builtin(&mut self, name: &str, num_params: usize, body: &str) {
        self.builtins.insert(
            name.to_string(),
            MacroDef {
                num_params,
                body: body.to_string(),
                optional_param: None,
            },
        );
    }

    /// Parse a macro definition from \newcommand or \renewcommand
    pub fn parse_macro_definition(
        &self,
        input: &str,
        _is_renew: bool,
    ) -> Result<(String, MacroDef)> {
        let mut chars = input.chars().peekable();
        
        // Skip whitespace
        while chars.peek() == Some(&' ') || chars.peek() == Some(&'\t') {
            chars.next();
        }

        // Parse macro name
        if chars.next() != Some('\\') {
            return Err(TexedError::InvalidSyntax(
                "Macro name must start with \\".to_string(),
            ));
        }

        let mut name = String::new();
        while let Some(&ch) = chars.peek() {
            if ch.is_alphabetic() {
                name.push(ch);
                chars.next();
            } else {
                break;
            }
        }

        if name.is_empty() {
            return Err(TexedError::InvalidSyntax(
                "Empty macro name".to_string(),
            ));
        }

        // Skip whitespace
        while chars.peek() == Some(&' ') || chars.peek() == Some(&'\t') {
            chars.next();
        }

        // Parse optional parameter count [n]
        let num_params = if chars.peek() == Some(&'[') {
            chars.next(); // consume '['
            let mut num_str = String::new();
            while let Some(&ch) = chars.peek() {
                if ch == ']' {
                    chars.next();
                    break;
                }
                num_str.push(ch);
                chars.next();
            }
            num_str.parse::<usize>().unwrap_or(0)
        } else {
            0
        };

        // Skip whitespace
        while chars.peek() == Some(&' ') || chars.peek() == Some(&'\t') {
            chars.next();
        }

        // Parse optional default parameter [default]
        let optional_param = if chars.peek() == Some(&'[') {
            chars.next(); // consume '['
            let mut default = String::new();
            while let Some(&ch) = chars.peek() {
                if ch == ']' {
                    chars.next();
                    break;
                }
                default.push(ch);
                chars.next();
            }
            Some(default)
        } else {
            None
        };

        // Skip whitespace
        while chars.peek() == Some(&' ') || chars.peek() == Some(&'\t') {
            chars.next();
        }

        // Parse body {body}
        if chars.next() != Some('{') {
            return Err(TexedError::InvalidSyntax(
                "Macro body must be enclosed in braces".to_string(),
            ));
        }

        let body = self.parse_balanced_braces(&chars.collect::<String>())?;

        Ok((
            name,
            MacroDef {
                num_params,
                body,
                optional_param,
            },
        ))
    }

    /// Parse balanced braces and return content
    fn parse_balanced_braces(&self, input: &str) -> Result<String> {
        let mut result = String::new();
        let mut depth = 1;
        let mut chars = input.chars();

        while let Some(ch) = chars.next() {
            match ch {
                '{' => {
                    depth += 1;
                    if depth > 1 {
                        result.push(ch);
                    }
                }
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(result);
                    }
                    result.push(ch);
                }
                '\\' => {
                    result.push(ch);
                    if let Some(next) = chars.next() {
                        result.push(next);
                    }
                }
                _ => result.push(ch),
            }
        }

        if depth != 0 {
            Err(TexedError::UnbalancedBraces)
        } else {
            Ok(result)
        }
    }

    /// Expand a macro with given arguments
    pub fn expand_macro(
        &self,
        state: &ParserState,
        name: &str,
        args: &[String],
    ) -> Option<String> {
        // Check user-defined macros first
        if let Some(expanded) = state.expand_macro(name, args) {
            return Some(expanded);
        }

        // Check built-in macros
        if let Some(def) = self.builtins.get(name) {
            let mut result = def.body.clone();
            
            // Replace parameters
            for (i, arg) in args.iter().enumerate() {
                let placeholder = format!("#{}", i + 1);
                result = result.replace(&placeholder, arg);
            }
            
            return Some(result);
        }

        None
    }

    /// Check if a macro is defined
    pub fn is_defined(&self, state: &ParserState, name: &str) -> bool {
        state.macros.contains_key(name) || self.builtins.contains_key(name)
    }

    pub fn get_definition<'a>(
        &'a self,
        state: &'a ParserState,
        name: &str,
    ) -> Option<&'a MacroDef> {
        state.macros.get(name).or_else(|| self.builtins.get(name))
    }

    /// Parse macro arguments from input
    pub fn parse_macro_args(
        &self,
        state: &ParserState,
        name: &str,
        input: &str,
    ) -> Result<(Vec<String>, usize)> {
        let def = state
            .macros
            .get(name)
            .or_else(|| self.builtins.get(name));

        let num_params = def.map(|d| d.num_params).unwrap_or(0);

        if num_params == 0 {
            return Ok((Vec::new(), 0));
        }

        let mut args = Vec::new();
        let mut chars = input.chars().peekable();
        let mut consumed = 0;

        // Skip whitespace
        while chars.peek() == Some(&' ') || chars.peek() == Some(&'\t') {
            chars.next();
            consumed += 1;
        }

        // Parse optional argument if defined
        if let Some(def) = def {
            if def.optional_param.is_some() && chars.peek() == Some(&'[') {
                chars.next(); // consume '['
                consumed += 1;
                
                let mut arg = String::new();
                let mut depth = 1;
                
                while let Some(ch) = chars.next() {
                    consumed += 1;
                    match ch {
                        '[' => {
                            depth += 1;
                            arg.push(ch);
                        }
                        ']' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                            arg.push(ch);
                        }
                        _ => arg.push(ch),
                    }
                }
                
                args.push(arg);
            } else if def.optional_param.is_some() {
                // Use default value
                args.push(def.optional_param.clone().unwrap());
            }
        }

        // Parse required arguments
        let required_params = if def.and_then(|d| d.optional_param.as_ref()).is_some() {
            num_params.saturating_sub(1)
        } else {
            num_params
        };

        for _ in 0..required_params {
            // Skip whitespace
            while chars.peek() == Some(&' ') || chars.peek() == Some(&'\t') {
                chars.next();
                consumed += 1;
            }

            if chars.peek() != Some(&'{') {
                return Err(TexedError::InvalidSyntax(
                    "Expected { for macro argument".to_string(),
                ));
            }

            chars.next(); // consume '{'
            consumed += 1;

            let mut arg = String::new();
            let mut depth = 1;

            while let Some(ch) = chars.next() {
                consumed += 1;
                match ch {
                    '{' => {
                        depth += 1;
                        arg.push(ch);
                    }
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                        arg.push(ch);
                    }
                    '\\' => {
                        arg.push(ch);
                        if let Some(next) = chars.next() {
                            consumed += 1;
                            arg.push(next);
                        }
                    }
                    _ => arg.push(ch),
                }
            }

            args.push(arg);
        }

        Ok((args, consumed))
    }
}

impl Default for MacroProcessor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_macros() {
        let processor = MacroProcessor::new();
        let state = ParserState::new();
        
        assert_eq!(
            processor.expand_macro(&state, "LaTeX", &[]),
            Some("LaTeX".to_string())
        );
        
        assert_eq!(
            processor.expand_macro(&state, "ie", &[]),
            Some("i.e.".to_string())
        );
    }

    #[test]
    fn test_macro_definition_parsing() {
        let processor = MacroProcessor::new();
        
        let result = processor.parse_macro_definition(
            "\\mycommand[2]{#1 and #2}",
            false,
        );
        
        assert!(result.is_ok());
        let (name, def) = result.unwrap();
        assert_eq!(name, "mycommand");
        assert_eq!(def.num_params, 2);
        assert_eq!(def.body, "#1 and #2");
    }

    #[test]
    fn test_macro_expansion() {
        let processor = MacroProcessor::new();
        let mut state = ParserState::new();
        
        state.define_macro(
            "mycommand".to_string(),
            2,
            "#1 and #2".to_string(),
        );
        
        let expanded = processor.expand_macro(
            &state,
            "mycommand",
            &["first".to_string(), "second".to_string()],
        );
        
        assert_eq!(expanded, Some("first and second".to_string()));
    }
}
