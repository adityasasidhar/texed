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
        .stdout(predicate::str::contains("0.2.0"));
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
fn test_author_year_citation_style() {
    let temp_dir = TempDir::new().unwrap();
    let input_file = temp_dir.path().join("input.tex");
    let output_file = temp_dir.path().join("output.md");
    let bib_file = temp_dir.path().join("refs.bib");

    let latex = r#"
As \citet{knuth84} showed, typesetting is hard \citep{lamport94}.

\bibliographystyle{plainnat}
\bibliography{refs}
"#;

    // Single-line entries exercise the character-level BibTeX parser
    let bibtex = concat!(
        "@book{knuth84, author = {Donald E. Knuth}, title = {The {TeX}book}, publisher = {Addison-Wesley}, year = {1984} }\n",
        "@book{lamport94, author = {Leslie Lamport}, title = {LaTeX}, publisher = {Addison-Wesley}, year = {1994} }\n",
    );

    fs::write(&input_file, latex).unwrap();
    fs::write(&bib_file, bibtex).unwrap();

    let mut cmd = Command::cargo_bin("texed").unwrap();
    cmd.arg(&input_file).arg("-o").arg(&output_file);
    cmd.assert().success();

    let output = fs::read_to_string(&output_file).unwrap();
    assert!(output.contains("Knuth (1984)"), "got: {}", output);
    assert!(output.contains("(Lamport, 1994)"), "got: {}", output);
    assert!(output.contains("## References"), "got: {}", output);
    assert!(output.contains("The TeXbook"), "got: {}", output);
}

#[test]
fn test_batch_conversion_writes_sibling_md_files() {
    let temp_dir = TempDir::new().unwrap();
    let a = temp_dir.path().join("a.tex");
    let b = temp_dir.path().join("b.tex");
    fs::write(&a, "\\section{Alpha}").unwrap();
    fs::write(&b, "\\section{Beta}").unwrap();

    let mut cmd = Command::cargo_bin("texed").unwrap();
    cmd.arg(&a).arg(&b);
    cmd.assert().success();

    let out_a = fs::read_to_string(temp_dir.path().join("a.md")).unwrap();
    let out_b = fs::read_to_string(temp_dir.path().join("b.md")).unwrap();
    assert!(out_a.contains("## Alpha"));
    assert!(out_b.contains("## Beta"));
}

#[test]
fn test_batch_conversion_into_directory() {
    let temp_dir = TempDir::new().unwrap();
    let a = temp_dir.path().join("a.tex");
    let b = temp_dir.path().join("b.tex");
    let out_dir = temp_dir.path().join("out");
    fs::write(&a, "one").unwrap();
    fs::write(&b, "two").unwrap();

    let mut cmd = Command::cargo_bin("texed").unwrap();
    cmd.arg(&a).arg(&b).arg("-o").arg(&out_dir);
    cmd.assert().success();

    assert!(out_dir.join("a.md").exists());
    assert!(out_dir.join("b.md").exists());
}

#[test]
fn test_batch_continues_past_failures_and_exits_nonzero() {
    let temp_dir = TempDir::new().unwrap();
    let good = temp_dir.path().join("good.tex");
    let missing = temp_dir.path().join("missing.tex");
    fs::write(&good, "fine").unwrap();

    let mut cmd = Command::cargo_bin("texed").unwrap();
    cmd.arg(&good).arg(&missing);
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("1 of 2 file(s) failed"));

    // The good file still converted
    assert!(temp_dir.path().join("good.md").exists());
}

#[test]
fn test_check_mode_reports_warnings_and_exits_nonzero() {
    let temp_dir = TempDir::new().unwrap();
    let input_file = temp_dir.path().join("input.tex");
    fs::write(&input_file, "Uses \\weirdcmd here. See \\ref{sec:nowhere}.").unwrap();

    let mut cmd = Command::cargo_bin("texed").unwrap();
    cmd.arg(&input_file).arg("--check");
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("unknown command `\\weirdcmd`"))
        .stderr(predicate::str::contains("unresolved reference"));

    // --check must not write output
    assert!(!temp_dir.path().join("input.md").exists());
}

#[test]
fn test_check_mode_clean_file_exits_zero() {
    let temp_dir = TempDir::new().unwrap();
    let input_file = temp_dir.path().join("input.tex");
    fs::write(&input_file, "\\section{Clean}\nJust \\textbf{fine} text.").unwrap();

    let mut cmd = Command::cargo_bin("texed").unwrap();
    cmd.arg(&input_file).arg("--check");
    cmd.assert().success();
}

#[test]
fn test_strict_mode_fails_on_warnings() {
    let temp_dir = TempDir::new().unwrap();
    let input_file = temp_dir.path().join("input.tex");
    let output_file = temp_dir.path().join("output.md");
    fs::write(&input_file, "Uses \\weirdcmd here.").unwrap();

    let mut cmd = Command::cargo_bin("texed").unwrap();
    cmd.arg(&input_file).arg("-o").arg(&output_file).arg("--strict");
    cmd.assert().failure();

    // Output is still written (strict fails the run, not the conversion)
    assert!(output_file.exists());
}

#[test]
fn test_no_frontmatter_flag() {
    let temp_dir = TempDir::new().unwrap();
    let input_file = temp_dir.path().join("input.tex");
    fs::write(&input_file, "\\title{My Doc}\nBody text.").unwrap();

    let mut cmd = Command::cargo_bin("texed").unwrap();
    cmd.arg(&input_file).arg("--no-frontmatter");
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("Body text."))
        .stdout(predicate::str::contains("---").not());
}

#[test]
fn test_quiet_suppresses_warnings() {
    let temp_dir = TempDir::new().unwrap();
    let input_file = temp_dir.path().join("input.tex");
    fs::write(&input_file, "Uses \\weirdcmd here.").unwrap();

    let mut cmd = Command::cargo_bin("texed").unwrap();
    cmd.arg(&input_file).arg("--quiet");
    cmd.assert()
        .success()
        .stderr(predicate::str::contains("warning").not());
}

#[test]
fn test_stats_flag_prints_statistics() {
    let temp_dir = TempDir::new().unwrap();
    let input_file = temp_dir.path().join("input.tex");
    let output_file = temp_dir.path().join("output.md");
    fs::write(&input_file, "\\section{One}\nText with $x$ math.").unwrap();

    let mut cmd = Command::cargo_bin("texed").unwrap();
    cmd.arg(&input_file).arg("-o").arg(&output_file).arg("--stats");
    cmd.assert()
        .success()
        .stderr(predicate::str::contains("headings"))
        .stderr(predicate::str::contains("time"));
}

#[test]
fn test_completions_generation() {
    for shell in ["bash", "zsh", "fish"] {
        let mut cmd = Command::cargo_bin("texed").unwrap();
        cmd.arg("--completions").arg(shell);
        cmd.assert()
            .success()
            .stdout(predicate::str::contains("texed"));
    }
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
