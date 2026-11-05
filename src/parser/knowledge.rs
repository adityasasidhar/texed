//! Tables of LaTeX/TeX command knowledge: which commands are pure set-up
//! (consumed silently with their arguments), which are TeX primitives that
//! take unbraced "register" values, and Unicode equivalents of math symbols
//! that show up in text mode.

/// Argument shape of a command that produces no output.
#[derive(Debug, Clone, Copy)]
pub(super) struct Signature {
    /// Optional `[...]` arguments allowed before the braced ones.
    pub leading_optional: bool,
    /// Number of mandatory braced arguments.
    pub braced: usize,
    /// Optional `[...]` arguments allowed after the braced ones.
    pub trailing_optional: bool,
}

const fn sig(leading_optional: bool, braced: usize, trailing_optional: bool) -> Signature {
    Signature {
        leading_optional,
        braced,
        trailing_optional,
    }
}

/// Set-up commands (package configuration, class/package boilerplate,
/// diagnostics) that never produce text. Their arguments are consumed.
pub(super) fn setup_command(cmd: &str) -> Option<Signature> {
    Some(match cmd {
        // Package/class boilerplate
        "ProvidesPackage" | "ProvidesClass" | "ProvidesFile" | "NeedsTeXFormat" => sig(false, 1, true),
        "LoadClass" | "LoadClassWithOptions" => sig(true, 1, true),
        "DeclareOption" => sig(false, 2, false),
        "ExecuteOptions" | "PassOptionsToClass" => sig(false, 1, false),
        "ProcessOptions" | "ProcessKeyvalOptions" => sig(false, 0, false),
        "DeclareKeys" | "SetKeys" | "ProcessKeyOptions" => sig(true, 1, false),
        "define@key" => sig(false, 3, false),
        "setkeys" => sig(false, 2, false),
        "ExplSyntaxOn" | "ExplSyntaxOff" => sig(false, 0, false),

        // Diagnostics
        "typeout" | "message" | "wlog" | "errmessage" | "GenericInfo" | "showthe" => sig(false, 1, false),
        "PackageWarning" | "PackageInfo" | "ClassWarning" | "ClassInfo" | "PackageError"
        | "ClassError" | "GenericWarning" | "PackageWarningNoLine" | "ClassWarningNoLine"
        | "GenericError" => sig(false, 2, false),

        // Counters, lengths, boxes
        "newcounter" => sig(false, 1, true),
        "newlength" | "newsavebox" => sig(false, 1, false),
        "setlength" | "addtolength" | "settowidth" | "settoheight" | "settodepth" => sig(false, 2, false),
        "newaliascnt" | "letcountercounter" | "counterwithin" | "counterwithout" => sig(false, 2, false),
        "aliascntresetthe" => sig(false, 1, false),

        // Page/layout set-up
        "geometry" | "newgeometry" | "hypersetup" | "captionsetup" | "sisetup" | "lstset"
        | "setminted" | "usemintedstyle" | "tcbset" | "pgfplotsset" | "tikzset" | "setcitestyle"
        | "urlstyle" | "floatstyle" | "restylefloat" | "DeclareGraphicsExtensions"
        | "pagestyle" | "thispagestyle" | "pagenumbering" | "setstretch" | "linespread"
        | "usetikzlibrary" | "usepgfplotslibrary" | "theoremstyle" | "declaretheoremstyle"
        | "setmainlanguage" | "setotherlanguage" | "selectlanguage" | "setdefaultlanguage" | "newpagestyle" | "fancypagestyle" | "includeonly" | "excludecomment" | "includecomment" | "titlelabel" | "MakeShortVerb"
        | "DeleteShortVerb" | "DefineShortVerb" | "UndefineShortVerb"
        | "setlistdepth" | "SetLabelAlign" | "setmathfont" | "setsansfont"
        | "setromanfont" | "defaultfontfeatures" | "xspaceaddexceptions"
        | "DisableLigatures" | "nolinenumbers" | "linenumbers" | "hyphenation" | "special"
        | "pdfinfo" | "pdfcatalog" | "pdfcompresslevel" | "pdfobjcompresslevel"
        | "fontencoding" | "fontfamily" | "fontseries" | "fontshape" | "ExecuteBibliographyOptions" => sig(true, 1, false),
        "setlist" | "setitemize" | "setenumerate" | "setdescription" => sig(true, 1, false),
        "newlist" => sig(false, 3, true),
        "renewlist" => sig(false, 3, true),
        "titleformat" => sig(false, 2, false), // remaining groups handled loosely below
        "titlespacing" => sig(false, 4, true),
        "titlecontents" => sig(false, 6, true),
        "fancyhead" | "fancyfoot" | "fancyhf" | "lhead" | "chead" | "rhead" | "lfoot"
        | "cfoot" | "rfoot" | "markright" => sig(true, 1, false),
        "markboth" => sig(false, 2, false),
        "crefname" | "Crefname" => sig(false, 3, false),
        "crefformat" | "Crefformat" | "crefrangeformat" | "Crefrangeformat" | "crefalias"
        | "creflabelformat" => sig(false, 2, false),
        "crefmultiformat" | "Crefmultiformat" | "crefrangemultiformat" | "Crefrangemultiformat" => sig(false, 5, false),
        "newtheoremstyle" => sig(false, 9, false),
        "newcolumntype" => sig(false, 1, false), // spec follows as [n]{...}; handled below
        "lstdefinestyle" | "lstdefinelanguage" | "colorlet" => sig(false, 2, false),
        "definecolor" | "providecolor" => sig(true, 3, false),
        "DeclareUnicodeCharacter" | "newunicodechar" | "DeclareSymbolFontAlphabet" => sig(false, 2, false),
        "DeclareMathAlphabet" | "DeclareSymbolFont" => sig(false, 5, false),
        "SetSymbolFont" | "DeclareFontShape" => sig(false, 6, true),
        "DeclareFontFamily" | "DeclareMathSymbol" => sig(false, 4, false),
        "DeclareFontSubstitution" => sig(false, 4, false),
        "DeclareMathDelimiter" => sig(false, 6, false),
        "newfontfamily" | "setmonofont" | "newfontface" => sig(true, 2, true),
        "setmainfont" => sig(true, 1, true),
        "newfloat" => sig(false, 3, true),
        "floatname" | "floatplacement" => sig(false, 2, false),
        "newsiamthm" | "newsiamremark" => sig(false, 2, false),
        "DeclareBibliographyCategory" | "defbibheading" => sig(true, 1, true),
        "DeclareFieldFormat" | "DeclareNameAlias" => sig(true, 2, false),
        "AtEveryBibitem" | "AtEveryCitekey" => sig(false, 1, false),
        "renewbibmacro" | "newbibmacro" => sig(false, 2, false),
        "glsaddall" | "makeglossaries" | "makenoidxglossaries" | "printglossaries"
        | "printnoidxglossaries" | "makeindex" | "printindex" | "listoffigures"
        | "listoftables" | "listoflistings" | "lstlistoflistings" | "listofalgorithms"
        | "ProcessOptions*" | "restoregeometry" | "phantomsection" | "FloatBarrier"
        | "clearpage" | "cleardoublepage" | "balance"
        | "IEEEpeerreviewmaketitle" | "IEEEdisplaynontitleabstractindextext"
        | "IEEEoverridecommandlockouts"
        | "frontmatter" | "mainmatter" | "backmatter" | "raggedcolumns" | "flushcolumns" | "nointerlineskip" | "offinterlineskip" | "normalbaselines" | "sloppy" | "fussy" | "centering" | "raggedright" | "raggedleft"
        | "endgraf" | "begingroup" | "endgroup" | "bgroup" | "egroup"
        | "noindent" | "indent" | "hfil" | "hfill" | "vfil" | "vfill" | "hss" | "vss"
        | "filbreak" | "goodbreak" | "eject" | "supereject" | "dosupereject" | "cr"
        | "crcr" | "null" | "empty" | "relax" | "unskip"
        | "unpenalty" | "unkern" | "leavevmode" | "smallbreak" | "medbreak" | "bigbreak"
        | "allowbreak" | "nobreak" | "break" | "newpage" | "strut" | "ignorespaces" | "ignorespacesafterend" | "protect" | "tracingall" | "tracingonline"
        | "loggingall" | "showlists" | "showboxdepth" | "errorstopmode" | "nonstopmode"
        | "batchmode" | "scrollmode" | "selectfont" | "normalfont"
        | "normalcolor" | "rmfamily" | "sffamily" | "ttfamily" | "mdseries" | "bfseries"
        | "upshape" | "itshape" | "slshape" | "scshape" | "nopagecolor" | "onecolumn"
        | "tightlist" | "tableofcontents" | "RaggedRight" | "RaggedLeft"
        | "Centering" | "justifying" | "UseRawInputEncoding" => sig(false, 0, false),
        "DocumentMetadata" | "PreventPackageFromLoading" => sig(false, 1, false),
        "pdfbookmark" | "bookmark" | "currentpdfbookmark" | "belowpdfbookmark" => sig(true, 2, false),
        "addcontentsline" => sig(false, 3, false),
        "addtocontents" => sig(false, 2, false),
        "index" | "glossary"
        | "indexentry" => sig(true, 1, false),
        "nomenclature" => sig(true, 2, false),
        "vspace" | "hspace" | "addvspace" | "enlargethispage" | "linethickness" => sig(false, 1, false),
        "vspace*" | "hspace*" => sig(false, 1, false),
        "rule" => sig(true, 2, false),
        "pagecolor" | "color" | "arrayrulecolor" => sig(true, 1, false),
        "rowcolors" => sig(true, 3, false),
        "AtBeginEnvironment" | "AtEndEnvironment" | "BeforeBeginEnvironment"
        | "AfterEndEnvironment" => sig(false, 2, false),
        "AtBeginShipout" | "AtEndDocument" | "AtBeginDvi" | "AddToShipoutPicture"
        | "AddToShipoutPictureBG" | "AddToShipoutPictureFG" => sig(false, 1, false),
        "AddToHook" | "AddToHookNext" => sig(false, 1, true), // {hook}[label]{code}: code consumed below
        "pretocmd" | "apptocmd" => sig(false, 4, false),
        "patchcmd" => sig(false, 5, false),
        "preto" | "appto" | "gappto" | "gpreto" | "eappto" | "xappto" => sig(false, 2, false),
        "csdef" | "csgdef" | "csedef" | "csxdef" => sig(false, 2, false),
        "bibpunct" => sig(true, 6, false),
        "acsetup" | "glsdisablehyper" | "setacronymstyle" | "setglossarystyle" => sig(false, 1, false),
        "ccsdesc" | "acmConference" | "acmBooktitle" | "acmDOI"
        | "acmISBN" | "acmYear" | "acmPrice" | "copyrightyear" | "acmJournal" | "acmVolume"
        | "acmNumber" | "acmArticle" | "acmMonth" | "setcopyright" | "settopmatter"
        | "received" | "revised" | "accepted" | "authorrunning" | "titlerunning"
        | "shorttitle" | "shortauthors" | "runningtitle" | "runningauthor" | "journal"
        | "volume" | "issue" | "pubyear" | "doi" | "subjclass" | "msc" | "jel" | "pacs"
        | "PACS" | "conference" | "titlehead" | "subject"
        | "publishers" | "uppertitleback" | "lowertitleback"
        | "extratitle" | "editor" | "orcid" | "orcidID"
        | "corref" | "cortext" | "fntext" | "tnotetext" | "ead" | "titlenote" | "subtitlenote" | "authornote" | "additionalaffiliation"
        | "IEEEspecialpapernotice" | "IEEEaftertitletext"
        | "renewcaptionname" | "addto" | "captionsetup*" => sig(true, 1, false),
        _ => return None,
    })
}

/// TeX primitives/registers that take an unbraced value (`\parskip 0pt`,
/// `\catcode`\@=11`, `\advance\count by 2`): `(control-sequence operands,
/// consumes a value spec)`.
pub(super) fn tex_assignment(cmd: &str) -> Option<(usize, bool)> {
    Some(match cmd {
        // Dimension/glue/integer registers and parameters
        "parskip" | "parindent" | "baselineskip" | "lineskip" | "lineskiplimit" | "hsize"
        | "vsize" | "textwidth" | "textheight" | "linewidth" | "columnwidth" | "columnsep"
        | "topmargin" | "oddsidemargin" | "evensidemargin" | "headheight" | "headsep"
        | "footskip" | "marginparwidth" | "marginparsep" | "topskip" | "maxdepth"
        | "leftskip" | "rightskip" | "parfillskip" | "hangindent" | "hangafter"
        | "tolerance" | "pretolerance" | "emergencystretch" | "hbadness" | "vbadness"
        | "hfuzz" | "vfuzz" | "hyphenpenalty" | "exhyphenpenalty" | "binoppenalty"
        | "relpenalty" | "clubpenalty" | "widowpenalty" | "displaywidowpenalty"
        | "brokenpenalty" | "predisplaypenalty" | "postdisplaypenalty" | "interlinepenalty"
        | "floatingpenalty" | "looseness" | "abovedisplayskip" | "belowdisplayskip"
        | "abovedisplayshortskip" | "belowdisplayshortskip" | "jot" | "mathsurround"
        | "arraycolsep" | "tabcolsep" | "arrayrulewidth" | "doublerulesep"
        | "fboxsep" | "fboxrule" | "itemsep" | "parsep" | "topsep" | "partopsep"
        | "labelsep" | "labelwidth" | "leftmargin" | "rightmargin" | "itemindent"
        | "listparindent" | "leftmargini" | "leftmarginii" | "leftmarginiii"
        | "footnotesep" | "escapechar" | "endlinechar" | "newlinechar"
        | "language" | "lefthyphenmin" | "righthyphenmin" | "uchyph" | "globaldefs"
        | "showboxbreadth" | "tracingmacros" | "tracingcommands" | "tracingparagraphs" | "tracingpages" | "tracingrestores"
        | "tracingstats" | "tracingoutput" | "tracinglostchars" | "time" | "day" | "month"
        | "year" | "mag" | "voffset" | "hoffset" | "pdfpagewidth" | "pdfpageheight"
        | "pagewidth" | "pageheight" | "paperwidth" | "paperheight" | "pdfminorversion" | "pdfadjustspacing" | "pdfprotrudechars" | "pdfoutput"
        | "spaceskip" | "xspaceskip" | "spacefactor" | "boxmaxdepth"
        | "splittopskip" | "splitmaxdepth" | "delimiterfactor" | "delimitershortfall"
        | "nulldelimiterspace" | "scriptspace" | "defaulthyphenchar"
        | "defaultskewchar" | "overfullrule" | "holdinginserts" | "outputpenalty"
        | "maxdeadcycles" | "deadcycles" | "insertpenalties" | "prevdepth" | "unitlength" | "skip" | "dimen" | "count" | "toks" | "muskip" | "fontdimen" | "pdfpkresolution" | "pdfimageresolution"
        | "pdfinclusionerrorlevel" | "pdfsuppresswarningpagegroup" | "interfootnotelinepenalty" | "medmuskip" | "thinmuskip" | "thickmuskip"
        | "linepenalty" | "adjdemerits" | "doublehyphendemerits" | "finalhyphendemerits"
        | "exhyphenchar" | "hyphenchar" | "skewchar" | "lccode" | "uccode" | "sfcode"
        | "mathcode" | "delcode" | "catcode" => (0, true),
        // \advance\x by 2, \multiply, \divide
        "advance" | "multiply" | "divide" => (1, true),
        // Register allocation: one control sequence
        "newcount" | "newdimen" | "newskip" | "newmuskip" | "newtoks" | "newbox" | "newread"
        | "newwrite" | "newinsert" | "newlanguage" | "newfam" => (1, false),
        // \chardef\x=65, \font\x=cmr10 at 10pt
        "chardef" | "mathchardef" | "countdef" | "dimendef" | "skipdef" | "toksdef"
        | "font" | "openin" | "openout" | "closein" | "closeout" | "read" => (1, true),
        // Spacing and penalties with an unbraced amount
        "kern" | "hskip" | "vskip" | "mskip" | "mkern" | "penalty"
        | "moveleft" | "moveright" | "lower" | "raise" | "hglue" | "vglue" => (0, true),
        _ => return None,
    })
}

/// Unicode for math-symbol commands used outside math mode (typically via
/// macros like `\newcommand{\lif}{\ensuremath{\to}}` or in old documents).
pub(super) fn text_math_symbol(cmd: &str) -> Option<&'static str> {
    Some(match cmd {
        "alpha" => "α", "beta" => "β", "gamma" => "γ", "delta" => "δ", "epsilon" => "ϵ",
        "varepsilon" => "ε", "zeta" => "ζ", "eta" => "η", "theta" => "θ", "vartheta" => "ϑ",
        "iota" => "ι", "kappa" => "κ", "lambda" => "λ", "mu" => "μ", "nu" => "ν", "xi" => "ξ",
        "pi" => "π", "varpi" => "ϖ", "rho" => "ρ", "varrho" => "ϱ", "sigma" => "σ",
        "varsigma" => "ς", "tau" => "τ", "upsilon" => "υ", "phi" => "ϕ", "varphi" => "φ",
        "chi" => "χ", "psi" => "ψ", "omega" => "ω", "Gamma" => "Γ", "Delta" => "Δ",
        "Theta" => "Θ", "Lambda" => "Λ", "Xi" => "Ξ", "Pi" => "Π", "Sigma" => "Σ",
        "Upsilon" => "Υ", "Phi" => "Φ", "Psi" => "Ψ", "Omega" => "Ω",
        "land" | "wedge" => "∧", "lor" | "vee" => "∨", "lnot" | "neg" => "¬",
        "forall" => "∀", "exists" => "∃", "nexists" => "∄", "to" | "rightarrow" => "→",
        "leftarrow" | "gets" => "←", "leftrightarrow" => "↔", "Rightarrow" | "implies" => "⇒",
        "Leftarrow" => "⇐", "Leftrightarrow" | "iff" => "⇔", "mapsto" => "↦",
        "longrightarrow" => "⟶", "longleftarrow" => "⟵", "Longrightarrow" => "⟹",
        "uparrow" => "↑", "downarrow" => "↓", "infty" => "∞", "partial" => "∂",
        "nabla" => "∇", "in" => "∈", "notin" => "∉", "ni" => "∋", "subset" => "⊂",
        "supset" => "⊃", "subseteq" => "⊆", "supseteq" => "⊇", "cup" => "∪", "cap" => "∩",
        "emptyset" | "varnothing" => "∅", "setminus" => "∖", "times" => "×", "div" => "÷",
        "pm" => "±", "mp" => "∓", "cdot" => "⋅", "circ" => "∘", "bullet" => "∙",
        "star" => "⋆", "ast" => "∗", "oplus" => "⊕", "otimes" => "⊗", "leq" | "le" => "≤",
        "geq" | "ge" => "≥", "neq" | "ne" => "≠", "approx" => "≈", "equiv" => "≡",
        "sim" => "∼", "simeq" => "≃", "cong" => "≅", "propto" => "∝", "ll" => "≪",
        "gg" => "≫", "perp" => "⊥", "parallel" => "∥", "mid" => "∣", "top" => "⊤",
        "bot" => "⊥", "vdash" => "⊢", "dashv" => "⊣", "models" => "⊨", "sum" => "∑",
        "prod" => "∏", "int" => "∫", "oint" => "∮", "sqrt" => "√", "angle" => "∠",
        "triangle" => "△", "Box" | "square" => "□", "Diamond" | "lozenge" => "◇",
        "langle" => "⟨", "rangle" => "⟩", "lceil" => "⌈", "rceil" => "⌉", "lfloor" => "⌊",
        "rfloor" => "⌋", "aleph" => "ℵ", "hbar" => "ℏ", "ell" => "ℓ", "wp" => "℘",
        "Re" => "ℜ", "Im" => "ℑ", "prime" => "′", "cdots" | "ldots" => "…",
        "vdots" => "⋮", "ddots" => "⋱", "colon" => ":", "vert" => "|", "Vert" => "‖",
        "lbrace" => "{", "rbrace" => "}", "backslash" => "\\", "smile" => "⌣",
        "frown" => "⌢", "uplus" => "⊎", "sqcup" => "⊔", "sqcap" => "⊓", "dagger" => "†",
        "ddagger" => "‡", "flat" => "♭", "natural" => "♮", "sharp" => "♯",
        "clubsuit" => "♣", "diamondsuit" => "♢", "heartsuit" => "♡", "spadesuit" => "♠",
        "checkmark" => "✓", "therefore" => "∴", "because" => "∵", "qed" | "qedsymbol" => "∎",
        "blacksquare" => "■", "lhd" => "⊲", "rhd" => "⊳", "unlhd" => "⊴", "unrhd" => "⊵",
        "bigcup" => "⋃", "bigcap" => "⋂", "bigoplus" => "⨁", "bigotimes" => "⨂",
        "bigvee" => "⋁", "bigwedge" => "⋀", "coprod" => "∐", "hookrightarrow" => "↪",
        "rightleftharpoons" => "⇌", "leadsto" => "⇝", "nearrow" => "↗", "searrow" => "↘",
        "Join" | "bowtie" => "⋈", "preceq" => "⪯", "succeq" => "⪰", "prec" => "≺",
        "succ" => "≻", "sqsubseteq" => "⊑", "sqsupseteq" => "⊒", "Vdash" => "⊩",
        "nvdash" => "⊬", "nmid" => "∤", "lessdot" => "⋖", "deg" => "deg",
        _ => return None,
    })
}

/// LaTeX documentation markup (ltxdoc/doc/l3doc), common in package docs.
pub(super) enum DocMarkup {
    /// Rendered as inline code of the argument
    Code,
    /// Rendered as inline code of `\argument`
    ControlSequence,
    /// Rendered as `⟨arg⟩`, wrapped in the given delimiters
    Meta(&'static str, &'static str),
}

pub(super) fn doc_markup(cmd: &str) -> Option<DocMarkup> {
    Some(match cmd {
        "cs" | "tn" | "csn" | "cmdname" => DocMarkup::ControlSequence,
        "pkg" | "cls" | "env" | "opt" | "file" | "option" | "class"
        | "package" | "Lpack" | "Lopt" | "Lenv" | "filename" | "fmtname" | "progname" => {
            DocMarkup::Code
        }
        "meta" | "metavar" => DocMarkup::Meta("⟨", "⟩"),
        "marg" => DocMarkup::Meta("{⟨", "⟩}"),
        "oarg" => DocMarkup::Meta("[⟨", "⟩]"),
        "parg" => DocMarkup::Meta("(⟨", "⟩)"),
        "Arg" | "arg" => DocMarkup::Meta("⟨", "⟩"),
        _ => return None,
    })
}

/// Common theorem-like environment names, used when a document uses one
/// without a visible `\newtheorem` (defined in a missing style file).
pub(super) fn implicit_theorem_name(env: &str) -> Option<&'static str> {
    let base = env.trim_end_matches('*');
    Some(match base {
        "thm" | "theorem" | "Theorem" | "thrm" | "theo" => "Theorem",
        "lem" | "lemma" | "Lemma" | "lemm" => "Lemma",
        "prop" | "proposition" | "Proposition" | "propo" => "Proposition",
        "cor" | "corollary" | "Corollary" | "coro" | "corol" => "Corollary",
        "defn" | "definition" | "Definition" | "defi" | "dfn" | "def" | "defin" => "Definition",
        "rmk" | "remark" | "Remark" | "rem" => "Remark",
        "ex" | "example" | "Example" | "exa" | "eg" | "examp" => "Example",
        "exercise" | "Exercise" | "exer" | "exr" | "ex2" => "Exercise",
        "prob" | "problem" | "Problem" | "prb" => "Problem",
        "conj" | "conjecture" | "Conjecture" => "Conjecture",
        "claim" | "Claim" | "clm" => "Claim",
        "fact" | "Fact" => "Fact",
        "obs" | "observation" | "Observation" => "Observation",
        "note" | "Note" => "Note",
        "notation" | "Notation" | "notn" => "Notation",
        "assumption" | "Assumption" | "assum" | "asm" => "Assumption",
        "hypothesis" | "Hypothesis" | "hyp" => "Hypothesis",
        "question" | "Question" | "qn" => "Question",
        "answer" | "Answer" => "Answer",
        "solution" | "Solution" | "sol" | "soln" => "Solution",
        "axiom" | "Axiom" | "ax" => "Axiom",
        "criterion" | "Criterion" => "Criterion",
        "principle" | "Principle" => "Principle",
        "summary" | "Summary" => "Summary",
        "case" | "Case" => "Case",
        "property" | "Property" | "prty" => "Property",
        "construction" | "Construction" | "constr" => "Construction",
        "convention" | "Convention" | "conventionnonum" => "Convention",
        "warning" | "Warning" => "Warning",
        "hint" | "Hint" => "Hint",
        "intuition" | "Intuition" => "Intuition",
        "exmp" => "Example",
        "proofsketch" | "sketch" => "Proof sketch",
        "digress" | "digression" => "Digression",
        "aside" => "Aside",
        "editorial" => "Editorial note",
        "defish" => "Definition",
        "remarkdefinition" => "Remark",
        _ => return None,
    })
}
