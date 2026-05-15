use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

#[test]
fn test_help_command() {
    let mut cmd = Command::cargo_bin("texed").unwrap();
    cmd.arg("--help");
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("convert LaTeX documents to Markdown format"));
}

#[test]
fn test_version_command() {
    let mut cmd = Command::cargo_bin("texed").unwrap();
    cmd.arg("--version");
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("0.1.0"));
}

#[test]
fn test_simple_text_conversion() {
    let temp_dir = TempDir::new().unwrap();
    let input_file = temp_dir.path().join("input.tex");
    let output_file = temp_dir.path().join("output.md");

    fs::write(&input_file, "Hello World").unwrap();

    let mut cmd = Command::cargo_bin("texed").unwrap();
    cmd.arg(&input_file)
        .arg("-o")
        .arg(&output_file);
    
    cmd.assert().success();

    let output = fs::read_to_string(&output_file).unwrap();
    assert!(output.contains("Hello World"));
}

#[test]
fn test_section_conversion() {
    let temp_dir = TempDir::new().unwrap();
    let input_file = temp_dir.path().join("input.tex");
    let output_file = temp_dir.path().join("output.md");

    fs::write(&input_file, "\\section{Introduction}\nThis is the introduction.").unwrap();

    let mut cmd = Command::cargo_bin("texed").unwrap();
    cmd.arg(&input_file)
        .arg("-o")
        .arg(&output_file);
    
    cmd.assert().success();

    let output = fs::read_to_string(&output_file).unwrap();
    assert!(output.contains("## Introduction"));
    assert!(output.contains("This is the introduction"));
}

#[test]
fn test_emphasis_conversion() {
    let temp_dir = TempDir::new().unwrap();
    let input_file = temp_dir.path().join("input.tex");
    let output_file = temp_dir.path().join("output.md");

    fs::write(&input_file, "\\textbf{bold} and \\textit{italic}").unwrap();

    let mut cmd = Command::cargo_bin("texed").unwrap();
    cmd.arg(&input_file)
        .arg("-o")
        .arg(&output_file);
    
    cmd.assert().success();

    let output = fs::read_to_string(&output_file).unwrap();
    assert!(output.contains("**bold**"));
    assert!(output.contains("*italic*"));
}

#[test]
fn test_list_conversion() {
    let temp_dir = TempDir::new().unwrap();
    let input_file = temp_dir.path().join("input.tex");
    let output_file = temp_dir.path().join("output.md");

    let latex = r#"\begin{itemize}
\item First item
\item Second item
\end{itemize}"#;

    fs::write(&input_file, latex).unwrap();

    let mut cmd = Command::cargo_bin("texed").unwrap();
    cmd.arg(&input_file)
        .arg("-o")
        .arg(&output_file);
    
    cmd.assert().success();

    let output = fs::read_to_string(&output_file).unwrap();
    assert!(output.contains("- First item"));
    assert!(output.contains("- Second item"));
}

#[test]
fn test_math_conversion() {
    let temp_dir = TempDir::new().unwrap();
    let input_file = temp_dir.path().join("input.tex");
    let output_file = temp_dir.path().join("output.md");

    fs::write(&input_file, "Inline $x + y$ and display $$E = mc^2$$").unwrap();

    let mut cmd = Command::cargo_bin("texed").unwrap();
    cmd.arg(&input_file)
        .arg("-o")
        .arg(&output_file);
    
    cmd.assert().success();

    let output = fs::read_to_string(&output_file).unwrap();
    assert!(output.contains("$x + y$"));
    assert!(output.contains("$$"));
    assert!(output.contains("E = mc^2"));
}

#[test]
fn test_stdin_stdout() {
    let mut cmd = Command::cargo_bin("texed").unwrap();
    cmd.write_stdin("Hello from stdin");
    
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("Hello from stdin"));
}

#[test]
fn test_force_overwrite() {
    let temp_dir = TempDir::new().unwrap();
    let input_file = temp_dir.path().join("input.tex");
    let output_file = temp_dir.path().join("output.md");

    fs::write(&input_file, "First content").unwrap();
    fs::write(&output_file, "Existing content").unwrap();

    // Without force, should fail
    let mut cmd = Command::cargo_bin("texed").unwrap();
    cmd.arg(&input_file)
        .arg("-o")
        .arg(&output_file);
    
    cmd.assert().failure();

    // With force, should succeed
    let mut cmd = Command::cargo_bin("texed").unwrap();
    cmd.arg(&input_file)
        .arg("-o")
        .arg(&output_file)
        .arg("--force");
    
    cmd.assert().success();

    let output = fs::read_to_string(&output_file).unwrap();
    assert!(output.contains("First content"));
}

#[test]
fn test_verbose_output() {
    let temp_dir = TempDir::new().unwrap();
    let input_file = temp_dir.path().join("input.tex");
    let output_file = temp_dir.path().join("output.md");

    fs::write(&input_file, "Test content").unwrap();

    let mut cmd = Command::cargo_bin("texed").unwrap();
    cmd.arg(&input_file)
        .arg("-o")
        .arg(&output_file)
        .arg("--verbose");
    
    cmd.assert()
        .success()
        .stderr(predicate::str::contains("Read"))
        .stderr(predicate::str::contains("bytes"));
}

#[test]
fn test_code_block_conversion() {
    let temp_dir = TempDir::new().unwrap();
    let input_file = temp_dir.path().join("input.tex");
    let output_file = temp_dir.path().join("output.md");

    let latex = r#"\begin{verbatim}
fn main() {
    println!("Hello");
}
\end{verbatim}"#;

    fs::write(&input_file, latex).unwrap();

    let mut cmd = Command::cargo_bin("texed").unwrap();
    cmd.arg(&input_file)
        .arg("-o")
        .arg(&output_file);
    
    cmd.assert().success();

    let output = fs::read_to_string(&output_file).unwrap();
    assert!(output.contains("```"));
    assert!(output.contains("fn main()"));
}

#[test]
fn test_bibliography_conversion_with_relative_bib_file() {
    let temp_dir = TempDir::new().unwrap();
    let input_file = temp_dir.path().join("input.tex");
    let output_file = temp_dir.path().join("output.md");
    let bib_file = temp_dir.path().join("refs.bib");

    let latex = r#"
This follows prior work \cite{knuth1984}.

\bibliographystyle{plain}
\bibliography{refs}
"#;

    let bibtex = r#"@book{knuth1984,
  author = {Donald E. Knuth},
  title = {The TeXbook},
  publisher = {Addison-Wesley},
  year = {1984}
}"#;

    fs::write(&input_file, latex).unwrap();
    fs::write(&bib_file, bibtex).unwrap();

    let mut cmd = Command::cargo_bin("texed").unwrap();
    cmd.arg(&input_file)
        .arg("-o")
        .arg(&output_file);

    cmd.assert().success();

    let output = fs::read_to_string(&output_file).unwrap();
    assert!(output.contains("[1]"));
    assert!(output.contains("## References"));
    assert!(output.contains("Donald E. Knuth"));
    assert!(output.contains("The TeXbook"));
}

#[test]
fn test_input_file_is_parsed_recursively() {
    let temp_dir = TempDir::new().unwrap();
    let input_file = temp_dir.path().join("main.tex");
    let child_file = temp_dir.path().join("child.tex");
    let output_file = temp_dir.path().join("output.md");

    fs::write(
        &input_file,
        "\\section{Main}\n\\input{child}\n",
    )
    .unwrap();
    fs::write(&child_file, "Child text with \\textbf{formatting}.").unwrap();

    let mut cmd = Command::cargo_bin("texed").unwrap();
    cmd.arg(&input_file).arg("-o").arg(&output_file);
    cmd.assert().success();

    let output = fs::read_to_string(&output_file).unwrap();
    assert!(output.contains("## Main"));
    assert!(output.contains("Child text with **formatting**."));
}

#[test]
fn test_newcommand_macro_expansion() {
    let temp_dir = TempDir::new().unwrap();
    let input_file = temp_dir.path().join("input.tex");
    let output_file = temp_dir.path().join("output.md");

    fs::write(
        &input_file,
        "\\newcommand{\\pair}[2]{#1 and #2}\n\\pair{alpha}{beta}",
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("texed").unwrap();
    cmd.arg(&input_file).arg("-o").arg(&output_file);
    cmd.assert().success();

    let output = fs::read_to_string(&output_file).unwrap();
    assert!(output.contains("alpha and beta"));
}

#[test]
fn test_newtheorem_custom_environment_is_rendered() {
    let temp_dir = TempDir::new().unwrap();
    let input_file = temp_dir.path().join("input.tex");
    let output_file = temp_dir.path().join("output.md");

    fs::write(
        &input_file,
        "\\newtheorem{remark}{Remark}\n\\begin{remark}[Key]\nCustom theorem body.\n\\end{remark}\n",
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("texed").unwrap();
    cmd.arg(&input_file).arg("-o").arg(&output_file);
    cmd.assert().success();

    let output = fs::read_to_string(&output_file).unwrap();
    assert!(output.contains("> **Remark 1 (Key)**"));
    assert!(output.contains("> Custom theorem body."));
}

#[test]
fn test_newtheorem_star_is_unnumbered() {
    let temp_dir = TempDir::new().unwrap();
    let input_file = temp_dir.path().join("input.tex");
    let output_file = temp_dir.path().join("output.md");

    fs::write(
        &input_file,
        "\\newtheorem*{remark}{Remark}\n\\begin{remark}\nUnnumbered body.\n\\end{remark}\n",
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("texed").unwrap();
    cmd.arg(&input_file).arg("-o").arg(&output_file);
    cmd.assert().success();

    let output = fs::read_to_string(&output_file).unwrap();
    assert!(output.contains("> **Remark**"));
    assert!(!output.contains("> **Remark 1**"));
}

#[test]
fn test_newtheorem_shared_counter_is_respected() {
    let temp_dir = TempDir::new().unwrap();
    let input_file = temp_dir.path().join("input.tex");
    let output_file = temp_dir.path().join("output.md");

    fs::write(
        &input_file,
        "\\newtheorem{thm}{Theorem}\n\\newtheorem{lem}[thm]{Lemma}\n\\begin{thm}A.\\end{thm}\n\\begin{lem}B.\\end{lem}\n",
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("texed").unwrap();
    cmd.arg(&input_file).arg("-o").arg(&output_file);
    cmd.assert().success();

    let output = fs::read_to_string(&output_file).unwrap();
    assert!(output.contains("> **Theorem 1**"));
    assert!(output.contains("> **Lemma 2**"));
}

#[test]
fn test_theorem_label_ref_and_autoref_are_resolved() {
    let temp_dir = TempDir::new().unwrap();
    let input_file = temp_dir.path().join("input.tex");
    let output_file = temp_dir.path().join("output.md");

    fs::write(
        &input_file,
        "\\newtheorem{remark}{Remark}\n\\begin{remark}\\label{rem:key}Body.\\end{remark}\nSee \\ref{rem:key} and \\autoref{rem:key}.\n",
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("texed").unwrap();
    cmd.arg(&input_file).arg("-o").arg(&output_file);
    cmd.assert().success();

    let output = fs::read_to_string(&output_file).unwrap();
    assert!(output.contains("> **Remark 1**"));
    assert!(output.contains("{#rem:key}"));
    assert!(output.contains("See 1 and Remark 1."));
}

#[test]
fn test_newtheorem_section_scoped_numbering_is_supported() {
    let temp_dir = TempDir::new().unwrap();
    let input_file = temp_dir.path().join("input.tex");
    let output_file = temp_dir.path().join("output.md");

    fs::write(
        &input_file,
        "\\newtheorem{thm}{Theorem}[section]\n\\section{One}\n\\begin{thm}A.\\end{thm}\n\\begin{thm}B.\\end{thm}\n\\section{Two}\n\\begin{thm}C.\\end{thm}\n",
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("texed").unwrap();
    cmd.arg(&input_file).arg("-o").arg(&output_file);
    cmd.assert().success();

    let output = fs::read_to_string(&output_file).unwrap();
    assert!(output.contains("> **Theorem 1.1**"));
    assert!(output.contains("> **Theorem 1.2**"));
    assert!(output.contains("> **Theorem 2.1**"));
}

#[test]
fn test_figure_label_autoref_is_resolved() {
    let temp_dir = TempDir::new().unwrap();
    let input_file = temp_dir.path().join("input.tex");
    let output_file = temp_dir.path().join("output.md");

    fs::write(
        &input_file,
        "\\begin{figure}\n\\includegraphics{plot.png}\n\\caption{Overview}\n\\label{fig:overview}\n\\end{figure}\nSee \\autoref{fig:overview}.\n",
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("texed").unwrap();
    cmd.arg(&input_file).arg("-o").arg(&output_file);
    cmd.assert().success();

    let output = fs::read_to_string(&output_file).unwrap();
    assert!(output.contains("![Overview](plot.png) {#fig:overview}"));
    assert!(output.contains("See Figure 1."));
}

#[test]
fn test_providecommand_does_not_override_existing_macro() {
    let temp_dir = TempDir::new().unwrap();
    let input_file = temp_dir.path().join("input.tex");
    let output_file = temp_dir.path().join("output.md");

    fs::write(
        &input_file,
        "\\newcommand{\\name}{Alice}\n\\providecommand{\\name}{Bob}\n\\name\n",
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("texed").unwrap();
    cmd.arg(&input_file).arg("-o").arg(&output_file);
    cmd.assert().success();

    let output = fs::read_to_string(&output_file).unwrap();
    assert!(output.contains("Alice"));
    assert!(!output.contains("Bob"));
}

#[test]
fn test_unknown_environment_is_preserved_as_raw_block() {
    let temp_dir = TempDir::new().unwrap();
    let input_file = temp_dir.path().join("input.tex");
    let output_file = temp_dir.path().join("output.md");

    fs::write(
        &input_file,
        "\\begin{mystery}\nopaque content\n\\end{mystery}\n",
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("texed").unwrap();
    cmd.arg(&input_file).arg("-o").arg(&output_file);
    cmd.assert().success();

    let output = fs::read_to_string(&output_file).unwrap();
    assert!(output.contains("\\begin{mystery}"));
    assert!(output.contains("opaque content"));
    assert!(output.contains("\\end{mystery}"));
}
