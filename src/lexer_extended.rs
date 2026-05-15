use std::collections::HashMap;

/// Extended token types for comprehensive LaTeX support
#[derive(Debug, Clone, PartialEq)]
pub enum ExtendedToken {
    // Citation commands
    Cite { command: String, keys: Vec<String>, prenote: Option<String>, postnote: Option<String> },
    Bibliography(String),
    BibResource(String),
    
    // Footnote commands
    Footnote(String),
    FootnoteMark(Option<usize>),
    FootnoteText { num: usize, text: String },
    
    // Macro definitions
    NewCommand { name: String, num_args: usize, optional: Option<String>, body: String },
    RenewCommand { name: String, num_args: usize, optional: Option<String>, body: String },
    NewEnvironment { name: String, num_args: usize, begin_def: String, end_def: String },
    
    // Conditional processing
    NewToggle(String),
    ToggleTrue(String),
    ToggleFalse(String),
    IfToggle { toggle: String, true_branch: String, false_branch: String },
    
    // Include system
    Include(String),
    Input(String),
    UsePackage { name: String, options: Vec<String> },
    
    // Accents and special characters
    Accent { accent_type: AccentType, char: char },
    SpecialChar(SpecialCharType),
    
    // Color
    TextColor { color: String, text: String },
    ColorBox { color: String, text: String },
    
    // Language
    SetLanguage(String),
    OtherLanguage { lang: String, content: String },
    
    // Metadata
    Title(String),
    Subtitle(String),
    Author(Vec<String>),
    Date(String),
    Thanks(String),
    
    // KOMA-Script metadata
    Subject(String),
    Publishers(String),
    Dedication(String),
    
    // Beamer
    FrameTitle(String),
    FrameSubtitle(String),
    Alert(String),
    
    // Box commands
    MBox(String),
    HBox(String),
    VBox(String),
    ParBox { width: String, content: String },
    
    // Spacing
    HSpace(String),
    VSpace(String),
    
    // Smart quotes
    LeftSingleQuote,
    RightSingleQuote,
    LeftDoubleQuote,
    RightDoubleQuote,
    
    // Case conversion
    MakeUppercase(String),
    MakeLowercase(String),
    
    // Lettrine (drop caps)
    Lettrine { letter: String, text: String },
    
    // Epigraph
    Epigraph { text: String, source: String },
    
    // Today
    Today,
    
    // Hyperlinks
    Hyperlink { target: String, text: String },
    Hypertarget { name: String, text: String },
    
    // URL
    Url(String),
    NoLinkUrl(String),
    
    // Graphics options
    GraphicsPath(Vec<String>),
    IncludeSvg { path: String, options: HashMap<String, String> },
    
    // Subfigure
    SubFigure { caption: Option<String>, content: String },
    
    // Advanced list
    SetCounter { counter: String, value: usize },
    
    // SIunitx
    SI { value: String, unit: String },
    Num(String),
    Unit(String),
    
    // Acronyms
    Acronym { short: String, long: String },
    
    // Raw pass-through
    PassThrough(String),
    
    // Horizontal rules
    Rule { width: String, height: String },
    PlainBreak,
    FancyBreak,
    
    // Obeylines
    ObeyLines(String),
    
    // File contents
    FileContents { filename: String, content: String },
}

#[derive(Debug, Clone, PartialEq)]
pub enum AccentType {
    Acute,      // \'
    Grave,      // \`
    Circumflex, // \^
    Tilde,      // \~
    Diaeresis,  // \"
    Macron,     // \=
    Dot,        // \.
    Breve,      // \u
    Caron,      // \v
    DoubleAcute,// \H
    Cedilla,    // \c
    Ogonek,     // \k
    Ring,       // \r
    Tie,        // \t
}

#[derive(Debug, Clone, PartialEq)]
pub enum SpecialCharType {
    // Dashes
    EnDash,     // --
    EmDash,     // ---
    
    // Spaces
    NonBreakingSpace,  // ~
    ThinSpace,         // \,
    NegThinSpace,      // \!
    MedSpace,          // \:
    ThickSpace,        // \;
    QuadSpace,         // \quad
    QQuadSpace,        // \qquad
    
    // Symbols
    Copyright,         // \copyright
    Registered,        // \textregistered
    Trademark,         // \texttrademark
    Paragraph,         // \P or \paragraph (symbol)
    Section,           // \S
    Dagger,            // \dag
    DoubleDagger,      // \ddag
    Bullet,            // \textbullet
    
    // Punctuation
    Ellipsis,          // \ldots or \dots
    
    // Currency
    Pound,             // \pounds
    Euro,              // \euro
    Dollar,            // \$
    
    // Math in text
    Degree,            // \degree
    Prime,             // \'
    DoublePrime,       // \'\'
    
    // Ligatures
    AE,                // \AE
    Ae,                // \ae
    OE,                // \OE
    Oe,                // \oe
    AA,                // \AA
    Aa,                // \aa
    O,                 // \O
    Oo,                // \o
    L,                 // \L
    Ll,                // \l
    SS,                // \ss
    
    // Quotes
    LeftGuillemet,     // \guillemotleft
    RightGuillemet,    // \guillemotright
}

/// Citation command types
pub const CITATION_COMMANDS: &[&str] = &[
    "cite", "citep", "citet", "citealt", "citealp",
    "citeauthor", "citeyear", "citeyearpar",
    "Cite", "Citep", "Citet", "Citealt", "Citealp",
    "Citeauthor",
    "autocite", "autocites", "textcite", "parencite",
    "footcite", "footcitetext",
    "smartcite", "supercite",
    "cites", "textcites", "parencites",
    "footcites", "footcitetexts", "smartcites", "supercites",
];

/// Reference command types
pub const REFERENCE_COMMANDS: &[&str] = &[
    "ref", "eqref", "autoref", "nameref", "pageref",
    "Ref", "Eqref", "Autoref", "Nameref", "Pageref",
    "cref", "Cref", "cpageref", "Cpageref",
];

/// Enquote command types
pub const ENQUOTE_COMMANDS: &[&str] = &[
    "enquote", "enquote*",
    "foreignquote", "foreignquote*",
    "hyphenquote", "hyphenquote*",
];

/// Verb command types
pub const VERB_COMMANDS: &[&str] = &[
    "verb", "verb*", "Verb",
    "lstinline",
];

/// Accent command mapping
pub fn get_accent_type(cmd: &str) -> Option<AccentType> {
    match cmd {
        "'" => Some(AccentType::Acute),
        "`" => Some(AccentType::Grave),
        "^" => Some(AccentType::Circumflex),
        "~" => Some(AccentType::Tilde),
        "\"" => Some(AccentType::Diaeresis),
        "=" => Some(AccentType::Macron),
        "." => Some(AccentType::Dot),
        "u" => Some(AccentType::Breve),
        "v" => Some(AccentType::Caron),
        "H" => Some(AccentType::DoubleAcute),
        "c" => Some(AccentType::Cedilla),
        "k" => Some(AccentType::Ogonek),
        "r" => Some(AccentType::Ring),
        "t" => Some(AccentType::Tie),
        _ => None,
    }
}

/// Special character command mapping
pub fn get_special_char(cmd: &str) -> Option<SpecialCharType> {
    match cmd {
        "copyright" => Some(SpecialCharType::Copyright),
        "textregistered" => Some(SpecialCharType::Registered),
        "texttrademark" => Some(SpecialCharType::Trademark),
        "P" => Some(SpecialCharType::Paragraph),
        "S" => Some(SpecialCharType::Section),
        "dag" => Some(SpecialCharType::Dagger),
        "ddag" => Some(SpecialCharType::DoubleDagger),
        "textbullet" => Some(SpecialCharType::Bullet),
        "ldots" | "dots" => Some(SpecialCharType::Ellipsis),
        "pounds" => Some(SpecialCharType::Pound),
        "euro" => Some(SpecialCharType::Euro),
        "degree" => Some(SpecialCharType::Degree),
        "AE" => Some(SpecialCharType::AE),
        "ae" => Some(SpecialCharType::Ae),
        "OE" => Some(SpecialCharType::OE),
        "oe" => Some(SpecialCharType::Oe),
        "AA" => Some(SpecialCharType::AA),
        "aa" => Some(SpecialCharType::Aa),
        "O" => Some(SpecialCharType::O),
        "o" => Some(SpecialCharType::Oo),
        "L" => Some(SpecialCharType::L),
        "l" => Some(SpecialCharType::Ll),
        "ss" => Some(SpecialCharType::SS),
        "guillemotleft" => Some(SpecialCharType::LeftGuillemet),
        "guillemotright" => Some(SpecialCharType::RightGuillemet),
        "," => Some(SpecialCharType::ThinSpace),
        "!" => Some(SpecialCharType::NegThinSpace),
        ":" => Some(SpecialCharType::MedSpace),
        ";" => Some(SpecialCharType::ThickSpace),
        "quad" => Some(SpecialCharType::QuadSpace),
        "qquad" => Some(SpecialCharType::QQuadSpace),
        _ => None,
    }
}

/// Math environment types
pub const MATH_ENVIRONMENTS: &[&str] = &[
    "equation", "equation*",
    "align", "align*",
    "alignat", "alignat*",
    "gather", "gather*",
    "multline", "multline*",
    "flalign", "flalign*",
    "split",
    "math", "displaymath",
    "eqnarray", "eqnarray*",
];

/// Theorem-like environment types
pub const THEOREM_ENVIRONMENTS: &[&str] = &[
    "theorem", "lemma", "proposition", "corollary",
    "definition", "example", "exercise", "remark",
    "proof", "claim", "conjecture", "axiom",
];

/// Table environment types
pub const TABLE_ENVIRONMENTS: &[&str] = &[
    "tabular", "tabular*", "tabularx",
    "longtable", "supertabular",
    "tabulary", "tabu",
    "array",
];

/// Code environment types
pub const CODE_ENVIRONMENTS: &[&str] = &[
    "verbatim", "verbatim*",
    "Verbatim", "BVerbatim",
    "lstlisting",
    "minted",
    "alltt",
    "code",
];

/// Quote environment types
pub const QUOTE_ENVIRONMENTS: &[&str] = &[
    "quote", "quotation", "verse",
    "displayquote",
];
