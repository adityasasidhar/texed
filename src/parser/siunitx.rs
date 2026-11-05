//! siunitx rendering: `\num{1.5e-3}` -> `1.5 × 10⁻³`, `\si{\kilo\gram\per
//! \second\squared}` -> `kg/s²`, `\SI{3}{\meter}` -> `3 m`, ...

/// Format a siunitx number: exponents become `× 10ⁿ`, `+-` becomes `±`.
pub(super) fn format_number(spec: &str) -> String {
    let spec = spec.trim().replace("\\pm", "±").replace("+-", "±").replace(['{', '}'], "");
    let spec = spec.replace("\\times", "×").replace("\\cdot", "·").replace('~', " ");
    // mantissa e exponent (also d/D/E as accepted by siunitx)
    if let Some(pos) = spec.find(['e', 'E', 'd', 'D']) {
        let (mantissa, exponent) = spec.split_at(pos);
        let exponent = exponent[1..].trim();
        let exponent_ok = !exponent.is_empty()
            && exponent
                .trim_start_matches(['+', '-'])
                .chars()
                .all(|c| c.is_ascii_digit());
        let mantissa_ok = mantissa
            .trim()
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '.' | ',' | '-' | '+' | '±' | ' ' | '(' | ')'));
        if exponent_ok && mantissa_ok {
            let exponent = exponent.trim_start_matches('+');
            let power = format!("10{}", superscript(exponent));
            let mantissa = mantissa.trim();
            return if mantissa.is_empty() {
                power
            } else {
                format!("{} × {}", mantissa, power)
            };
        }
    }
    spec.trim().to_string()
}

/// Accumulates units while scanning a siunitx unit specification.
#[derive(Default)]
struct UnitBuilder {
    numerator: Vec<String>,
    denominator: Vec<String>,
    prefix: String,
    per: bool,
    power: Option<String>,
}

impl UnitBuilder {
    fn push(&mut self, unit: &str) {
        let mut unit = format!("{}{}", self.prefix, unit);
        self.prefix.clear();
        if let Some(p) = self.power.take() {
            unit.push_str(&superscript(&p));
        }
        if self.per {
            self.denominator.push(unit);
            self.per = false;
        } else {
            self.numerator.push(unit);
        }
    }

    /// The unit most recently added (where a postfix power applies).
    fn last_mut(&mut self) -> Option<&mut String> {
        if self.denominator.is_empty() {
            self.numerator.last_mut()
        } else {
            self.denominator.last_mut()
        }
    }

    fn finish(self) -> String {
        let mut out = self.numerator.join("\u{2009}");
        if !self.denominator.is_empty() {
            if out.is_empty() {
                out.push('1');
            }
            out.push('/');
            if self.denominator.len() > 1 {
                out.push('(');
                out.push_str(&self.denominator.join("\u{2009}"));
                out.push(')');
            } else {
                out.push_str(&self.denominator[0]);
            }
        }
        out
    }
}

/// Format a siunitx unit specification (macro or literal form).
pub(super) fn format_unit(spec: &str) -> String {
    let spec = spec.trim();
    if !spec.contains('\\') {
        return format_literal_unit(spec);
    }
    let mut b = UnitBuilder::default();
    let chars: Vec<char> = spec.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '\\' {
            // a run of literal characters is one unit (`{pc}`, `px`)
            let start = i;
            while i < chars.len() && chars[i] != '\\' && !chars[i].is_whitespace() && !matches!(chars[i], '{' | '}' | '~' | '.') {
                i += 1;
            }
            if i > start {
                let unit: String = chars[start..i].iter().collect();
                b.push(&unit);
            } else {
                i += 1;
            }
            continue;
        }
        let start = i + 1;
        let mut end = start;
        while end < chars.len() && chars[end].is_alphabetic() {
            end += 1;
        }
        let name: String = chars[start..end].iter().collect();
        i = end.max(start + 1).min(chars.len().max(end));
        if end == start {
            // control symbol like \, : skip it
            i = (start + 1).min(chars.len());
            continue;
        }
        match name.as_str() {
            "per" => b.per = true,
            "square" => b.power = Some("2".into()),
            "cubic" => b.power = Some("3".into()),
            "squared" | "cubed" => {
                let p = if name == "squared" { "²" } else { "³" };
                if let Some(unit) = b.last_mut() {
                    unit.push_str(p);
                }
            }
            "tothe" | "raiseto" => {
                let (arg, next) = read_group(&chars, i);
                i = next;
                if name == "raiseto" {
                    b.power = Some(arg);
                } else if let Some(unit) = b.last_mut() {
                    unit.push_str(&superscript(&arg));
                }
            }
            "of" => {
                let (arg, next) = read_group(&chars, i);
                i = next;
                if let Some(unit) = b.last_mut() {
                    unit.push_str(&format!("({})", arg));
                }
            }
            "highlight" => {
                let (_, next) = read_group(&chars, i);
                i = next;
            }
            _ => {
                if let Some(p) = unit_prefix(&name) {
                    b.prefix.push_str(p);
                } else if let Some(u) = unit_symbol(&name) {
                    b.push(u);
                } else {
                    b.push(&name);
                }
            }
        }
    }
    b.finish()
}

/// `kg.m/s^2` style literal units.
fn format_literal_unit(spec: &str) -> String {
    let mut out = String::new();
    let mut chars = spec.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '.' | '*' => out.push('\u{00B7}'),
            '~' => out.push('\u{2009}'),
            '^' => {
                let mut exp = String::new();
                if chars.peek() == Some(&'{') {
                    chars.next();
                    for ch in chars.by_ref() {
                        if ch == '}' {
                            break;
                        }
                        exp.push(ch);
                    }
                } else {
                    if chars.peek() == Some(&'-') {
                        exp.push(chars.next().unwrap());
                    }
                    while let Some(&d) = chars.peek() {
                        if d.is_ascii_digit() {
                            exp.push(d);
                            chars.next();
                        } else {
                            break;
                        }
                    }
                }
                out.push_str(&superscript(&exp));
            }
            '{' | '}' => {}
            _ => out.push(c),
        }
    }
    out
}

fn read_group(chars: &[char], mut i: usize) -> (String, usize) {
    while i < chars.len() && chars[i].is_whitespace() {
        i += 1;
    }
    if i < chars.len() && chars[i] == '{' {
        let mut depth = 0;
        let start = i + 1;
        while i < chars.len() {
            match chars[i] {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return (chars[start..i].iter().collect(), i + 1);
                    }
                }
                _ => {}
            }
            i += 1;
        }
        (chars[start..].iter().collect(), chars.len())
    } else if i < chars.len() {
        (chars[i].to_string(), i + 1)
    } else {
        (String::new(), i)
    }
}

/// Unicode superscript form of an exponent (falls back to `^x`).
pub(super) fn superscript(exp: &str) -> String {
    let mut out = String::new();
    for c in exp.trim().chars() {
        out.push(match c {
            '0' => '⁰',
            '1' => '¹',
            '2' => '²',
            '3' => '³',
            '4' => '⁴',
            '5' => '⁵',
            '6' => '⁶',
            '7' => '⁷',
            '8' => '⁸',
            '9' => '⁹',
            '-' | '\u{2212}' => '⁻',
            '+' => '⁺',
            'n' => 'ⁿ',
            _ => return format!("^{}", exp.trim()),
        });
    }
    out
}

fn unit_prefix(name: &str) -> Option<&'static str> {
    Some(match name {
        "yocto" => "y", "zepto" => "z", "atto" => "a", "femto" => "f", "pico" => "p",
        "nano" => "n", "micro" => "µ", "milli" => "m", "centi" => "c", "deci" => "d",
        "deca" | "deka" => "da", "hecto" => "h", "kilo" => "k", "mega" => "M", "giga" => "G",
        "tera" => "T", "peta" => "P", "exa" => "E", "zetta" => "Z", "yotta" => "Y",
        "kibi" => "Ki", "mebi" => "Mi", "gibi" => "Gi", "tebi" => "Ti",
        _ => return None,
    })
}

fn unit_symbol(name: &str) -> Option<&'static str> {
    Some(match name {
        "metre" | "meter" => "m", "gram" => "g", "kilogram" => "kg", "second" => "s",
        "ampere" => "A", "kelvin" => "K", "mole" => "mol", "candela" => "cd",
        "hertz" => "Hz", "newton" => "N", "pascal" => "Pa", "joule" => "J", "watt" => "W",
        "coulomb" => "C", "volt" => "V", "farad" => "F", "ohm" => "Ω", "siemens" => "S",
        "weber" => "Wb", "tesla" => "T", "henry" => "H", "degreeCelsius" | "celsius" => "°C",
        "lumen" => "lm", "lux" => "lx", "becquerel" => "Bq", "gray" => "Gy",
        "sievert" => "Sv", "katal" => "kat", "minute" => "min", "hour" => "h", "day" => "d",
        "degree" => "°", "arcminute" => "′", "arcsecond" => "″", "hectare" => "ha",
        "litre" | "liter" => "L", "tonne" => "t", "electronvolt" => "eV", "dalton" => "Da",
        "astronomicalunit" => "au", "bel" => "B", "decibel" => "dB", "neper" => "Np",
        "bit" => "bit", "byte" => "B", "percent" => "%", "angstrom" | "Angstrom" => "Å",
        "bar" => "bar", "barn" => "b", "knot" => "kn", "mmHg" => "mmHg",
        "nauticalmile" => "M", "atomicmassunit" => "u", "bohr" => "a₀", "clight" => "c",
        "electronmass" => "mₑ", "elementarycharge" => "e", "hartree" => "Eₕ",
        "planckbar" => "ℏ", "radian" => "rad", "steradian" => "sr", "gauss" => "G",
        "calorie" => "cal", "molar" => "M", "inch" => "in", "foot" => "ft", "mile" => "mi",
        "pound" => "lb", "ounce" => "oz", "psi" => "psi", "rpm" => "rpm", "parsec" => "pc",
        "lightyear" => "ly", "erg" => "erg", "dyne" => "dyn", "poise" => "P",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers() {
        assert_eq!(format_number("1e5"), "1 × 10⁵");
        assert_eq!(format_number("1.5e-3"), "1.5 × 10⁻³");
        assert_eq!(format_number("12345"), "12345");
        assert_eq!(format_number("3.0 +- 0.1"), "3.0 ± 0.1");
        assert_eq!(format_number("e10"), "10¹⁰");
    }

    #[test]
    fn units() {
        assert_eq!(format_unit("\\meter\\per\\second"), "m/s");
        assert_eq!(format_unit("\\kilo\\gram\\meter\\per\\second\\squared"), "kg\u{2009}m/s²");
        assert_eq!(format_unit("\\square\\metre"), "m²");
        assert_eq!(format_unit("\\micro\\ohm"), "µΩ");
        assert_eq!(format_unit("kg.m/s^2"), "kg·m/s²");
        assert_eq!(format_unit("\\percent"), "%");
    }
}
