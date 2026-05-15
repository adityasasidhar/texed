use crate::error::{Result, TexedError};
use crate::state::ParserState;
use std::fs;
use std::path::{Path, PathBuf};

/// Include system for LaTeX with cycle detection and path resolution
#[derive(Clone)]
pub struct IncludeSystem {
    /// Search paths for included files (TEXINPUTS)
    search_paths: Vec<PathBuf>,
    
    /// Maximum include depth to prevent infinite recursion
    max_depth: usize,
    
    /// Current include depth
    current_depth: usize,
}

impl IncludeSystem {
    pub fn new() -> Self {
        let mut search_paths = vec![PathBuf::from(".")];
        
        // Add TEXINPUTS environment variable paths
        if let Ok(texinputs) = std::env::var("TEXINPUTS") {
            for path in texinputs.split(':') {
                if !path.is_empty() && path != "." {
                    search_paths.push(PathBuf::from(path));
                }
            }
        }
        
        Self {
            search_paths,
            max_depth: 10,
            current_depth: 0,
        }
    }

    /// Add a search path
    pub fn add_search_path(&mut self, path: PathBuf) {
        if !self.search_paths.contains(&path) {
            self.search_paths.push(path);
        }
    }

    /// Set maximum include depth
    pub fn set_max_depth(&mut self, depth: usize) {
        self.max_depth = depth;
    }

    /// Include a file with \include command
    /// Adds .tex extension if not present
    pub fn include_file(
        &mut self,
        state: &mut ParserState,
        filename: &str,
        base_path: Option<&Path>,
    ) -> Result<String> {
        self.current_depth += 1;
        
        if self.current_depth > self.max_depth {
            self.current_depth -= 1;
            return Err(TexedError::InvalidSyntax(
                format!("Maximum include depth ({}) exceeded", self.max_depth),
            ));
        }

        let result = self.read_file(state, filename, base_path, true);
        
        self.current_depth -= 1;
        result
    }

    /// Input a file with \input command
    /// Does not automatically add .tex extension
    pub fn input_file(
        &mut self,
        state: &mut ParserState,
        filename: &str,
        base_path: Option<&Path>,
    ) -> Result<String> {
        self.current_depth += 1;
        
        if self.current_depth > self.max_depth {
            self.current_depth -= 1;
            return Err(TexedError::InvalidSyntax(
                format!("Maximum include depth ({}) exceeded", self.max_depth),
            ));
        }

        let result = self
            .read_file(state, filename, base_path, false)
            .or_else(|_| self.read_file(state, filename, base_path, true));
        
        self.current_depth -= 1;
        result
    }

    /// Read a file with cycle detection
    fn read_file(
        &self,
        state: &mut ParserState,
        filename: &str,
        base_path: Option<&Path>,
        add_tex_extension: bool,
    ) -> Result<String> {
        // Resolve file path
        let resolved_path = self.resolve_path(filename, base_path, add_tex_extension)?;
        
        // Convert to canonical path for cycle detection
        let canonical_path = resolved_path
            .canonicalize()
            .unwrap_or_else(|_| resolved_path.clone());
        
        let path_str = canonical_path.to_string_lossy().to_string();

        // Check for cycles
        if !state.can_include_file(&path_str) {
            return Err(TexedError::InvalidSyntax(
                format!("Circular include detected: {}", filename),
            ));
        }

        // Mark file as included
        state.mark_file_included(path_str.clone());

        // Read file content
        let content = fs::read_to_string(&resolved_path)
            .map_err(|e| TexedError::InputFileError(e))?;

        // Cache file content
        state.file_contents.insert(path_str, content.clone());

        Ok(content)
    }

    /// Resolve file path with search paths
    fn resolve_path(
        &self,
        filename: &str,
        base_path: Option<&Path>,
        add_tex_extension: bool,
    ) -> Result<PathBuf> {
        let mut candidates = Vec::new();

        // Try with original filename
        candidates.push(filename.to_string());

        // Try with .tex extension if needed
        if add_tex_extension && !filename.ends_with(".tex") {
            candidates.push(format!("{}.tex", filename));
        }

        // Try each candidate in each search path
        for candidate in &candidates {
            // Try relative to base path first
            if let Some(base) = base_path {
                let path = base.join(candidate);
                if path.exists() {
                    return Ok(path);
                }
            }

            // Try in search paths
            for search_path in &self.search_paths {
                let path = search_path.join(candidate);
                if path.exists() {
                    return Ok(path);
                }
            }
        }

        Err(TexedError::InputFileError(
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("File not found: {}", filename),
            ),
        ))
    }

    /// Load a package file (.sty)
    pub fn load_package(
        &mut self,
        state: &mut ParserState,
        package_name: &str,
        options: &[String],
    ) -> Result<Option<String>> {
        let filename = format!("{}.sty", package_name);
        
        // Try to find and read the package file
        match self.resolve_path(&filename, None, false) {
            Ok(path) => {
                let canonical_path = path
                    .canonicalize()
                    .unwrap_or_else(|_| path.clone());
                
                let path_str = canonical_path.to_string_lossy().to_string();

                // Check if already loaded
                if state.file_contents.contains_key(&path_str) {
                    return Ok(None);
                }

                // Read package content
                let content = fs::read_to_string(&path)
                    .map_err(|e| TexedError::InputFileError(e))?;

                // Cache package content
                state.file_contents.insert(path_str, content.clone());

                Ok(Some(content))
            }
            Err(_) => {
                // Package not found - this is often OK as many packages are built-in
                // Store package name and options in metadata
                state.metadata.insert(
                    format!("package:{}", package_name),
                    options.join(","),
                );
                Ok(None)
            }
        }
    }

    /// Read a subfile
    pub fn read_subfile(
        &mut self,
        state: &mut ParserState,
        filename: &str,
        base_path: Option<&Path>,
    ) -> Result<String> {
        // Subfile is similar to input but may have different handling
        self.input_file(state, filename, base_path)
    }

    /// Check if a file exists in search paths
    pub fn file_exists(&self, filename: &str, base_path: Option<&Path>) -> bool {
        self.resolve_path(filename, base_path, true).is_ok()
    }

    /// Get the resolved path for a file
    pub fn get_resolved_path(
        &self,
        filename: &str,
        base_path: Option<&Path>,
    ) -> Option<PathBuf> {
        self.resolve_path(filename, base_path, true).ok()
    }
}

impl Default for IncludeSystem {
    fn default() -> Self {
        Self::new()
    }
}

/// Handle \graphicspath command
pub fn parse_graphics_path(path_spec: &str) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let mut current_path = String::new();
    let mut in_braces = false;

    for ch in path_spec.chars() {
        match ch {
            '{' => {
                in_braces = true;
                current_path.clear();
            }
            '}' => {
                if in_braces && !current_path.is_empty() {
                    paths.push(PathBuf::from(current_path.trim()));
                    current_path.clear();
                }
                in_braces = false;
            }
            _ => {
                if in_braces {
                    current_path.push(ch);
                }
            }
        }
    }

    paths
}

/// Resolve graphics file with common extensions
pub fn resolve_graphics_file(
    filename: &str,
    graphics_paths: &[PathBuf],
    base_path: Option<&Path>,
) -> Option<PathBuf> {
    let extensions = ["", ".png", ".jpg", ".jpeg", ".pdf", ".eps", ".svg"];
    
    for ext in &extensions {
        let candidate = if ext.is_empty() {
            filename.to_string()
        } else {
            format!("{}{}", filename, ext)
        };

        // Try relative to base path
        if let Some(base) = base_path {
            let path = base.join(&candidate);
            if path.exists() {
                return Some(path);
            }
        }

        // Try in graphics paths
        for graphics_path in graphics_paths {
            let path = graphics_path.join(&candidate);
            if path.exists() {
                return Some(path);
            }
        }

        // Try in current directory
        let path = PathBuf::from(&candidate);
        if path.exists() {
            return Some(path);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_graphics_path() {
        let paths = parse_graphics_path("{./images/}{./figures/}");
        assert_eq!(paths.len(), 2);
        assert_eq!(paths[0], PathBuf::from("./images/"));
        assert_eq!(paths[1], PathBuf::from("./figures/"));
    }

    #[test]
    fn test_include_system_creation() {
        let system = IncludeSystem::new();
        assert!(system.search_paths.len() >= 1);
        assert_eq!(system.max_depth, 10);
    }

    #[test]
    fn test_add_search_path() {
        let mut system = IncludeSystem::new();
        let initial_count = system.search_paths.len();
        
        system.add_search_path(PathBuf::from("/tmp/test"));
        assert_eq!(system.search_paths.len(), initial_count + 1);
        
        // Adding same path again should not increase count
        system.add_search_path(PathBuf::from("/tmp/test"));
        assert_eq!(system.search_paths.len(), initial_count + 1);
    }
}
