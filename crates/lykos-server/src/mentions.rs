//! @Erwähnungen im Nachrichteninhalt finden und Emoji-Reaktionen prüfen.
//!
//! Erwähnungen in Code (```Block``` oder `inline`) zählen nicht – wie im Client-Markdown.

use std::collections::BTreeSet;

/// Höchstzahl verschiedener Namen, die pro Nachricht aufgelöst werden.
const MAX_CANDIDATES: usize = 50;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Parsed {
    /// Kleingeschriebene Benutzernamen-Kandidaten (zum Abgleich mit `lower(username)`).
    pub usernames: Vec<String>,
    /// `@everyone` kommt vor (wirkt nur mit `MENTION_EVERYONE`).
    pub everyone: bool,
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-')
}

/// Ersetzt Codeblöcke und Inline-Code durch Leerzeichen (nur mit schließendem Begrenzer).
fn strip_code(content: &str) -> String {
    let chars: Vec<char> = content.chars().collect();
    let mut out = String::with_capacity(content.len());
    let mut i = 0;
    let starts_with =
        |i: usize, pat: &str| pat.chars().enumerate().all(|(k, p)| chars.get(i + k) == Some(&p));
    while i < chars.len() {
        if starts_with(i, "```")
            && let Some(end) = (i + 3..chars.len()).find(|&j| starts_with(j, "```"))
        {
            out.push(' ');
            i = end + 3;
            continue;
        }
        if chars[i] == '`'
            && let Some(end) =
                (i + 1..chars.len()).take_while(|&j| chars[j] != '\n').find(|&j| chars[j] == '`')
        {
            out.push(' ');
            i = end + 1;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Sucht `@name` (Benutzernamen-Zeichen `[A-Za-z0-9_.-]`, 2–32) und `@everyone`.
/// Ein `@` direkt nach einem Namenszeichen (z. B. in E-Mail-Adressen) zählt nicht.
/// Satzzeichen am Ende (`@bob.`) werden zusätzlich abgeschnitten probiert.
pub fn parse(content: &str) -> Parsed {
    let text = strip_code(content);
    let chars: Vec<char> = text.chars().collect();
    let mut names = BTreeSet::new();
    let mut everyone = false;
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '@' || (i > 0 && is_name_char(chars[i - 1])) {
            i += 1;
            continue;
        }
        let start = i + 1;
        let mut end = start;
        while end < chars.len() && end - start < 33 && is_name_char(chars[end]) {
            end += 1;
        }
        let token: String = chars[start..end].iter().collect::<String>().to_lowercase();
        i = end.max(i + 1);
        if token == "everyone" {
            everyone = true;
            continue;
        }
        let trimmed = token.trim_end_matches(['.', '-']);
        for name in [token.as_str(), trimmed] {
            if (2..=32).contains(&name.chars().count()) && names.len() < MAX_CANDIDATES {
                names.insert(name.to_owned());
            }
        }
    }
    Parsed { usernames: names.into_iter().collect(), everyone }
}

/// Emoji-Codepunkte (grob): Symbole, Piktogramme, Flaggen, Modifikatoren und Verbinder.
fn is_emoji_char(c: char) -> bool {
    matches!(c as u32,
        0x00A9 | 0x00AE | 0x203C | 0x2049 | 0x2122 | 0x2139
        | 0x2194..=0x21AA
        | 0x231A..=0x23FF
        | 0x24C2
        | 0x25AA..=0x27BF
        | 0x2934..=0x2935
        | 0x2B05..=0x2B55
        | 0x3030 | 0x303D | 0x3297 | 0x3299
        | 0x1F000..=0x1FAFF
        | 0xE0020..=0xE007F
        | 0x200D | 0x20E3 | 0xFE0E | 0xFE0F)
}

/// Prüft eine Reaktion: kurzer String aus Emoji-Codepunkten, kein beliebiger Text.
/// Tastenkappen (`1️⃣`, `#️⃣`) sind erlaubt, sonst keine ASCII-Zeichen.
pub fn is_valid_emoji(emoji: &str) -> bool {
    let chars: Vec<char> = emoji.chars().collect();
    if chars.is_empty() || chars.len() > 16 || emoji.len() > 64 {
        return false;
    }
    // Erstes Zeichen darf kein reiner Verbinder/Modifikator sein.
    if matches!(chars[0] as u32, 0x200D | 0x20E3 | 0xFE0E | 0xFE0F | 0xE0020..=0xE007F) {
        return false;
    }
    let keycap = chars.contains(&'\u{20E3}');
    let has_pictograph =
        chars.iter().any(|&c| is_emoji_char(c) && !matches!(c as u32, 0x200D | 0xFE0E | 0xFE0F));
    has_pictograph
        && chars.iter().enumerate().all(|(i, &c)| {
            is_emoji_char(c) || (keycap && i == 0 && (c.is_ascii_digit() || c == '#' || c == '*'))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(content: &str) -> Vec<String> {
        parse(content).usernames
    }

    #[test]
    fn finds_mentions() {
        assert_eq!(names("Hallo @Bob und @carol_1!"), vec!["bob", "carol_1"]);
        assert_eq!(names("@bob."), vec!["bob", "bob."]);
        assert!(names("mail@example.org").is_empty());
        assert!(names("@ allein, @x").is_empty());
        assert!(parse("Alle mal herhören @everyone").everyone);
        assert!(!parse("@everyoneelse").everyone);
    }

    #[test]
    fn ignores_code() {
        assert!(names("`@bob` und ```\n@carol\n```").is_empty());
        assert_eq!(names("`offen @bob"), vec!["bob"]);
        assert!(!parse("`@everyone`").everyone);
    }

    #[test]
    fn validates_emoji() {
        for ok in ["👍", "❤️", "🇩🇪", "👨‍👩‍👧", "1️⃣", "✅", "👍🏽", "🏳️‍🌈"]
        {
            assert!(is_valid_emoji(ok), "{ok}");
        }
        for bad in
            ["", "a", "lol", "👍 ", "<script>", "1", "\u{FE0F}", "👍a", "👍👍👍👍👍👍👍👍👍👍👍👍👍👍👍👍👍"]
        {
            assert!(!is_valid_emoji(bad), "{bad:?}");
        }
    }
}
