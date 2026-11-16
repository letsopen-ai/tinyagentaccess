pub(super) fn ascii(c: char) -> Option<(u16, bool)> {
    let lower = c.to_ascii_lowercase();
    let hardware = "asdfhgzxcv bqweryt123465=97-80]ou[ip";
    if let Some(code) = hardware.chars().position(|x| x == lower) {
        if code != 10 {
            return Some((code as u16, c.is_ascii_uppercase()));
        }
    }
    let p = match c {
        ' ' => (49, false),
        '\'' => (39, false),
        '"' => (39, true),
        ';' => (41, false),
        ':' => (41, true),
        '\\' => (42, false),
        '|' => (42, true),
        ',' => (43, false),
        '<' => (43, true),
        '/' => (44, false),
        '?' => (44, true),
        'j' | 'J' => (38, c.is_ascii_uppercase()),
        'k' | 'K' => (40, c.is_ascii_uppercase()),
        'l' | 'L' => (37, c.is_ascii_uppercase()),
        'n' | 'N' => (45, c.is_ascii_uppercase()),
        'm' | 'M' => (46, c.is_ascii_uppercase()),
        '.' => (47, false),
        '>' => (47, true),
        '`' => (50, false),
        '~' => (50, true),
        '+' => (24, true),
        '_' => (27, true),
        '!' => (18, true),
        '@' => (19, true),
        '#' => (20, true),
        '$' => (21, true),
        '%' => (23, true),
        '^' => (22, true),
        '&' => (26, true),
        '*' => (28, true),
        '(' => (25, true),
        ')' => (29, true),
        '{' => (33, true),
        '}' => (30, true),
        _ => return None,
    };
    Some(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_printable_ascii_and_letter_case() {
        for c in ' '..='~' {
            assert!(ascii(c).is_some(), "{c}");
        }
        for (c, code) in [('j', 38), ('k', 40), ('l', 37), ('n', 45), ('m', 46)] {
            assert_eq!(ascii(c), Some((code, false)));
            assert_eq!(ascii(c.to_ascii_uppercase()), Some((code, true)));
        }
        for c in 'a'..='z' {
            assert_eq!(
                ascii(c.to_ascii_uppercase()),
                ascii(c).map(|(code, _)| (code, true))
            );
        }
    }
}
