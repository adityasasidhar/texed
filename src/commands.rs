use crate::error::Result;
use crate::lexer_extended::{AccentType, SpecialCharType};
use std::collections::HashMap;

/// Command handler registry for inline commands
#[derive(Clone)]
pub struct CommandRegistry {
    handlers: HashMap<String, CommandHandler>,
}

pub type CommandHandler = fn(&str, &[String]) -> Result<String>;

impl CommandRegistry {
    pub fn new() -> Self {
        let mut registry = Self {
            handlers: HashMap::new(),
        };
        registry.register_default_commands();
        registry
    }

    fn register_default_commands(&mut self) {
        // Text formatting
        self.register("textbf", handle_textbf);
        self.register("textit", handle_textit);
        self.register("texttt", handle_texttt);
        self.register("textsc", handle_textsc);
        self.register("emph", handle_emph);
        self.register("underline", handle_underline);
        
        // Font families
        self.register("textrm", handle_textrm);
        self.register("textsf", handle_textsf);
        self.register("textmd", handle_textmd);
        self.register("textup", handle_textup);
        self.register("textnormal", handle_textnormal);
        
        // Strikeout (soul/ulem)
        self.register("st", handle_strikeout);
        self.register("sout", handle_strikeout);
        
        // Underline variants
        self.register("ul", handle_underline);
        self.register("uline", handle_underline);
        
        // Highlight
        self.register("hl", handle_highlight);
        
        // Superscript/subscript
        self.register("textsuperscript", handle_superscript);
        self.register("textsubscript", handle_subscript);
        
        // Case conversion
        self.register("MakeUppercase", handle_uppercase);
        self.register("MakeTextUppercase", handle_uppercase);
        self.register("uppercase", handle_uppercase);
        self.register("MakeLowercase", handle_lowercase);
        self.register("MakeTextLowercase", handle_lowercase);
        self.register("lowercase", handle_lowercase);
        
        // URLs
        self.register("url", handle_url);
        self.register("nolinkurl", handle_nolinkurl);
        self.register("href", handle_href);
        
        // Box commands
        self.register("mbox", handle_mbox);
        self.register("hbox", handle_hbox);
        self.register("vbox", handle_vbox);
        
        // Special
        self.register("today", handle_today);
    }

    pub fn register(&mut self, name: &str, handler: CommandHandler) {
        self.handlers.insert(name.to_string(), handler);
    }

    pub fn contains(&self, name: &str) -> bool {
        self.handlers.contains_key(name)
    }

    pub fn handle(&self, name: &str, args: &[String]) -> Result<String> {
        if let Some(handler) = self.handlers.get(name) {
            handler(name, args)
        } else {
            Ok(format!("\\{}", name))
        }
    }
}

impl Default for CommandRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// Command handlers

fn handle_textbf(_cmd: &str, args: &[String]) -> Result<String> {
    Ok(format!("**{}**", args.first().unwrap_or(&String::new())))
}

fn handle_textit(_cmd: &str, args: &[String]) -> Result<String> {
    Ok(format!("*{}*", args.first().unwrap_or(&String::new())))
}

fn handle_texttt(_cmd: &str, args: &[String]) -> Result<String> {
    Ok(format!("`{}`", args.first().unwrap_or(&String::new())))
}

fn handle_textsc(_cmd: &str, args: &[String]) -> Result<String> {
    Ok(format!(
        "<span style=\"font-variant: small-caps;\">{}</span>",
        args.first().unwrap_or(&String::new())
    ))
}

fn handle_emph(_cmd: &str, args: &[String]) -> Result<String> {
    Ok(format!("*{}*", args.first().unwrap_or(&String::new())))
}

fn handle_underline(_cmd: &str, args: &[String]) -> Result<String> {
    Ok(format!("<u>{}</u>", args.first().unwrap_or(&String::new())))
}

fn handle_textrm(_cmd: &str, args: &[String]) -> Result<String> {
    Ok(format!(
        "<span class=\"roman\">{}</span>",
        args.first().unwrap_or(&String::new())
    ))
}

fn handle_textsf(_cmd: &str, args: &[String]) -> Result<String> {
    Ok(format!(
        "<span class=\"sans-serif\">{}</span>",
        args.first().unwrap_or(&String::new())
    ))
}

fn handle_textmd(_cmd: &str, args: &[String]) -> Result<String> {
    Ok(format!(
        "<span class=\"medium\">{}</span>",
        args.first().unwrap_or(&String::new())
    ))
}

fn handle_textup(_cmd: &str, args: &[String]) -> Result<String> {
    Ok(format!(
        "<span class=\"upright\">{}</span>",
        args.first().unwrap_or(&String::new())
    ))
}

fn handle_textnormal(_cmd: &str, args: &[String]) -> Result<String> {
    Ok(format!(
        "<span class=\"nodecor\">{}</span>",
        args.first().unwrap_or(&String::new())
    ))
}

fn handle_strikeout(_cmd: &str, args: &[String]) -> Result<String> {
    Ok(format!("~~{}~~", args.first().unwrap_or(&String::new())))
}

fn handle_highlight(_cmd: &str, args: &[String]) -> Result<String> {
    Ok(format!(
        "<mark>{}</mark>",
        args.first().unwrap_or(&String::new())
    ))
}

fn handle_superscript(_cmd: &str, args: &[String]) -> Result<String> {
    Ok(format!("<sup>{}</sup>", args.first().unwrap_or(&String::new())))
}

fn handle_subscript(_cmd: &str, args: &[String]) -> Result<String> {
    Ok(format!("<sub>{}</sub>", args.first().unwrap_or(&String::new())))
}

fn handle_uppercase(_cmd: &str, args: &[String]) -> Result<String> {
    Ok(args.first()
        .unwrap_or(&String::new())
        .to_uppercase())
}

fn handle_lowercase(_cmd: &str, args: &[String]) -> Result<String> {
    Ok(args.first()
        .unwrap_or(&String::new())
        .to_lowercase())
}

fn handle_url(_cmd: &str, args: &[String]) -> Result<String> {
    let empty = String::new();
    let url = args.first().unwrap_or(&empty);
    Ok(format!("[{}]({})", url, url))
}

fn handle_nolinkurl(_cmd: &str, args: &[String]) -> Result<String> {
    let empty = String::new();
    Ok(format!("`{}`", args.first().unwrap_or(&empty)))
}

fn handle_href(_cmd: &str, args: &[String]) -> Result<String> {
    let empty = String::new();
    let url = args.first().unwrap_or(&empty);
    let text = args.get(1).unwrap_or(url);
    Ok(format!("[{}]({})", text, url))
}

fn handle_mbox(_cmd: &str, args: &[String]) -> Result<String> {
    Ok(args.first().unwrap_or(&String::new()).clone())
}

fn handle_hbox(_cmd: &str, args: &[String]) -> Result<String> {
    Ok(args.first().unwrap_or(&String::new()).clone())
}

fn handle_vbox(_cmd: &str, args: &[String]) -> Result<String> {
    Ok(args.first().unwrap_or(&String::new()).clone())
}

fn handle_today(_cmd: &str, _args: &[String]) -> Result<String> {
    use chrono::Local;
    Ok(Local::now().format("%B %d, %Y").to_string())
}

/// Convert accent type and character to Unicode
pub fn apply_accent(accent: AccentType, ch: char) -> char {
    match (accent, ch) {
        // Acute accents
        (AccentType::Acute, 'a') => 'á',
        (AccentType::Acute, 'e') => 'é',
        (AccentType::Acute, 'i') => 'í',
        (AccentType::Acute, 'o') => 'ó',
        (AccentType::Acute, 'u') => 'ú',
        (AccentType::Acute, 'y') => 'ý',
        (AccentType::Acute, 'A') => 'Á',
        (AccentType::Acute, 'E') => 'É',
        (AccentType::Acute, 'I') => 'Í',
        (AccentType::Acute, 'O') => 'Ó',
        (AccentType::Acute, 'U') => 'Ú',
        (AccentType::Acute, 'Y') => 'Ý',
        
        // Grave accents
        (AccentType::Grave, 'a') => 'à',
        (AccentType::Grave, 'e') => 'è',
        (AccentType::Grave, 'i') => 'ì',
        (AccentType::Grave, 'o') => 'ò',
        (AccentType::Grave, 'u') => 'ù',
        (AccentType::Grave, 'A') => 'À',
        (AccentType::Grave, 'E') => 'È',
        (AccentType::Grave, 'I') => 'Ì',
        (AccentType::Grave, 'O') => 'Ò',
        (AccentType::Grave, 'U') => 'Ù',
        
        // Circumflex
        (AccentType::Circumflex, 'a') => 'â',
        (AccentType::Circumflex, 'e') => 'ê',
        (AccentType::Circumflex, 'i') => 'î',
        (AccentType::Circumflex, 'o') => 'ô',
        (AccentType::Circumflex, 'u') => 'û',
        (AccentType::Circumflex, 'A') => 'Â',
        (AccentType::Circumflex, 'E') => 'Ê',
        (AccentType::Circumflex, 'I') => 'Î',
        (AccentType::Circumflex, 'O') => 'Ô',
        (AccentType::Circumflex, 'U') => 'Û',
        
        // Tilde
        (AccentType::Tilde, 'a') => 'ã',
        (AccentType::Tilde, 'n') => 'ñ',
        (AccentType::Tilde, 'o') => 'õ',
        (AccentType::Tilde, 'A') => 'Ã',
        (AccentType::Tilde, 'N') => 'Ñ',
        (AccentType::Tilde, 'O') => 'Õ',
        
        // Diaeresis
        (AccentType::Diaeresis, 'a') => 'ä',
        (AccentType::Diaeresis, 'e') => 'ë',
        (AccentType::Diaeresis, 'i') => 'ï',
        (AccentType::Diaeresis, 'o') => 'ö',
        (AccentType::Diaeresis, 'u') => 'ü',
        (AccentType::Diaeresis, 'y') => 'ÿ',
        (AccentType::Diaeresis, 'A') => 'Ä',
        (AccentType::Diaeresis, 'E') => 'Ë',
        (AccentType::Diaeresis, 'I') => 'Ï',
        (AccentType::Diaeresis, 'O') => 'Ö',
        (AccentType::Diaeresis, 'U') => 'Ü',
        
        // Acute (extended)
        (AccentType::Acute, 'c') => 'ć',
        (AccentType::Acute, 'C') => 'Ć',
        (AccentType::Acute, 'n') => 'ń',
        (AccentType::Acute, 'N') => 'Ń',
        (AccentType::Acute, 's') => 'ś',
        (AccentType::Acute, 'S') => 'Ś',
        (AccentType::Acute, 'z') => 'ź',
        (AccentType::Acute, 'Z') => 'Ź',
        (AccentType::Acute, 'l') => 'ĺ',
        (AccentType::Acute, 'r') => 'ŕ',

        // Tilde (extended)
        (AccentType::Tilde, 'u') => 'ũ',
        (AccentType::Tilde, 'U') => 'Ũ',
        (AccentType::Tilde, 'i') => 'ĩ',
        (AccentType::Tilde, 'I') => 'Ĩ',

        // Ring
        (AccentType::Ring, 'a') => 'å',
        (AccentType::Ring, 'A') => 'Å',
        (AccentType::Ring, 'u') => 'ů',
        (AccentType::Ring, 'U') => 'Ů',

        // Cedilla
        (AccentType::Cedilla, 'c') => 'ç',
        (AccentType::Cedilla, 'C') => 'Ç',
        (AccentType::Cedilla, 's') => 'ş',
        (AccentType::Cedilla, 'S') => 'Ş',
        (AccentType::Cedilla, 't') => 'ţ',
        (AccentType::Cedilla, 'T') => 'Ţ',

        // Caron
        (AccentType::Caron, 'c') => 'č',
        (AccentType::Caron, 'C') => 'Č',
        (AccentType::Caron, 's') => 'š',
        (AccentType::Caron, 'S') => 'Š',
        (AccentType::Caron, 'z') => 'ž',
        (AccentType::Caron, 'Z') => 'Ž',
        (AccentType::Caron, 'r') => 'ř',
        (AccentType::Caron, 'R') => 'Ř',
        (AccentType::Caron, 'e') => 'ě',
        (AccentType::Caron, 'E') => 'Ě',
        (AccentType::Caron, 'd') => 'ď',
        (AccentType::Caron, 'D') => 'Ď',
        (AccentType::Caron, 't') => 'ť',
        (AccentType::Caron, 'T') => 'Ť',
        (AccentType::Caron, 'n') => 'ň',
        (AccentType::Caron, 'N') => 'Ň',

        // Double acute
        (AccentType::DoubleAcute, 'o') => 'ő',
        (AccentType::DoubleAcute, 'O') => 'Ő',
        (AccentType::DoubleAcute, 'u') => 'ű',
        (AccentType::DoubleAcute, 'U') => 'Ű',

        // Macron
        (AccentType::Macron, 'a') => 'ā',
        (AccentType::Macron, 'A') => 'Ā',
        (AccentType::Macron, 'e') => 'ē',
        (AccentType::Macron, 'E') => 'Ē',
        (AccentType::Macron, 'i') => 'ī',
        (AccentType::Macron, 'I') => 'Ī',
        (AccentType::Macron, 'o') => 'ō',
        (AccentType::Macron, 'O') => 'Ō',
        (AccentType::Macron, 'u') => 'ū',
        (AccentType::Macron, 'U') => 'Ū',

        // Breve
        (AccentType::Breve, 'g') => 'ğ',
        (AccentType::Breve, 'G') => 'Ğ',
        (AccentType::Breve, 'a') => 'ă',
        (AccentType::Breve, 'A') => 'Ă',
        (AccentType::Breve, 'u') => 'ŭ',
        (AccentType::Breve, 'U') => 'Ŭ',

        // Ogonek
        (AccentType::Ogonek, 'a') => 'ą',
        (AccentType::Ogonek, 'A') => 'Ą',
        (AccentType::Ogonek, 'e') => 'ę',
        (AccentType::Ogonek, 'E') => 'Ę',

        // Dot above
        (AccentType::Dot, 'z') => 'ż',
        (AccentType::Dot, 'Z') => 'Ż',
        (AccentType::Dot, 'e') => 'ė',
        (AccentType::Dot, 'E') => 'Ė',
        (AccentType::Dot, 'c') => 'ċ',
        (AccentType::Dot, 'C') => 'Ċ',
        (AccentType::Dot, 'g') => 'ġ',
        (AccentType::Dot, 'G') => 'Ġ',
        (AccentType::Dot, 'I') => 'İ',

        // Default: return original character
        _ => ch,
    }
}

/// Convert special character type to string
pub fn special_char_to_string(special: SpecialCharType) -> String {
    match special {
        SpecialCharType::EnDash => "–".to_string(),
        SpecialCharType::EmDash => "—".to_string(),
        SpecialCharType::NonBreakingSpace => " ".to_string(),
        SpecialCharType::ThinSpace => " ".to_string(),
        SpecialCharType::NegThinSpace => "".to_string(),
        SpecialCharType::MedSpace => " ".to_string(),
        SpecialCharType::ThickSpace => " ".to_string(),
        SpecialCharType::QuadSpace => "    ".to_string(),
        SpecialCharType::QQuadSpace => "        ".to_string(),
        SpecialCharType::Copyright => "©".to_string(),
        SpecialCharType::Registered => "®".to_string(),
        SpecialCharType::Trademark => "™".to_string(),
        SpecialCharType::Paragraph => "¶".to_string(),
        SpecialCharType::Section => "§".to_string(),
        SpecialCharType::Dagger => "†".to_string(),
        SpecialCharType::DoubleDagger => "‡".to_string(),
        SpecialCharType::Bullet => "•".to_string(),
        SpecialCharType::Ellipsis => "…".to_string(),
        SpecialCharType::Pound => "£".to_string(),
        SpecialCharType::Euro => "€".to_string(),
        SpecialCharType::Dollar => "$".to_string(),
        SpecialCharType::Degree => "°".to_string(),
        SpecialCharType::Prime => "′".to_string(),
        SpecialCharType::DoublePrime => "″".to_string(),
        SpecialCharType::AE => "Æ".to_string(),
        SpecialCharType::Ae => "æ".to_string(),
        SpecialCharType::OE => "Œ".to_string(),
        SpecialCharType::Oe => "œ".to_string(),
        SpecialCharType::AA => "Å".to_string(),
        SpecialCharType::Aa => "å".to_string(),
        SpecialCharType::O => "Ø".to_string(),
        SpecialCharType::Oo => "ø".to_string(),
        SpecialCharType::L => "Ł".to_string(),
        SpecialCharType::Ll => "ł".to_string(),
        SpecialCharType::SS => "ß".to_string(),
        SpecialCharType::LeftGuillemet => "«".to_string(),
        SpecialCharType::RightGuillemet => "»".to_string(),
    }
}
