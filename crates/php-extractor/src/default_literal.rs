//! Tells which constructor or method defaults can be read from source text.
//!
//! Anything else is a PHP expression, and only PHP knows its value.

/// True when `raw` is a literal whose value and type follow directly from its text.
pub fn is_plain_literal_default(raw: &str) -> bool {
    let text = raw.trim();
    let bytes = text.as_bytes();
    is_keyword(text)
        || is_plain_number(text)
        || is_whole_single_quoted(bytes)
        || is_whole_simple_double_quoted(bytes)
        || is_plain_array(text)
}

fn is_keyword(text: &str) -> bool {
    ["null", "true", "false"]
        .iter()
        .any(|keyword| text.eq_ignore_ascii_case(keyword))
}

/// A decimal integer that fits 64 bits for certain, or a decimal float. Octal, hex,
/// binary, separators and exponents are left to PHP.
fn is_plain_number(text: &str) -> bool {
    let digits = text.strip_prefix('-').unwrap_or(text);
    let (whole, fraction) = match digits.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (digits, None),
    };
    let all_digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    all_digits(whole)
        && (whole == "0" || !whole.starts_with('0'))
        && whole.len() <= 18
        && fraction.is_none_or(all_digits)
}

/// One single-quoted string from the first to the last byte, so `'a' . 'b'` is excluded.
fn is_whole_single_quoted(bytes: &[u8]) -> bool {
    if bytes.len() < 2 || bytes[0] != b'\'' {
        return false;
    }
    let mut i = 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'\'' => return i == bytes.len() - 1,
            _ => i += 1,
        }
    }
    false
}

/// Double quotes without escapes or interpolation mean the same as single quotes.
fn is_whole_simple_double_quoted(bytes: &[u8]) -> bool {
    bytes.len() >= 2
        && bytes[0] == b'"'
        && bytes[bytes.len() - 1] == b'"'
        && !bytes[1..bytes.len() - 1]
            .iter()
            .any(|b| matches!(b, b'"' | b'\\' | b'$'))
}

/// An array of plain ASCII strings, numbers, keywords and arrays, either a list or fully
/// keyed. A mix of the two needs PHP's key counting, and escapes or other bytes in strings
/// need PHP's string rules, so those go to PHP as well.
fn is_plain_array(text: &str) -> bool {
    let mut scan = ArrayScan {
        src: text.as_bytes(),
        pos: 0,
    };
    scan.array() && scan.at_end()
}

struct ArrayScan<'a> {
    src: &'a [u8],
    pos: usize,
}

impl ArrayScan<'_> {
    fn at_end(&mut self) -> bool {
        self.skip_ws();
        self.pos >= self.src.len()
    }

    fn skip_ws(&mut self) {
        while self.src.get(self.pos).is_some_and(u8::is_ascii_whitespace) {
            self.pos += 1;
        }
    }

    fn eat(&mut self, token: &[u8]) -> bool {
        self.skip_ws();
        if self.src[self.pos..].starts_with(token) {
            self.pos += token.len();
            return true;
        }
        false
    }

    fn opening(&mut self) -> Option<&'static [u8]> {
        if self.eat(b"[") {
            return Some(b"]");
        }
        self.skip_ws();
        let rest = &self.src[self.pos..];
        if rest.len() >= 5 && rest[..5].eq_ignore_ascii_case(b"array") {
            let start = self.pos;
            self.pos += 5;
            if self.eat(b"(") {
                return Some(b")");
            }
            self.pos = start;
        }
        None
    }

    fn array(&mut self) -> bool {
        let Some(close) = self.opening() else {
            return false;
        };
        let (mut keyed, mut listed) = (false, false);
        loop {
            if self.eat(close) {
                return true;
            }
            let start = self.pos;
            if !self.value() {
                return false;
            }
            if self.eat(b"=>") {
                keyed = true;
                if !is_key(&self.src[start..self.pos - 2]) || !self.value() {
                    return false;
                }
            } else {
                listed = true;
            }
            if keyed && listed {
                return false;
            }
            if !self.eat(b",") {
                return self.eat(close);
            }
        }
    }

    fn value(&mut self) -> bool {
        self.skip_ws();
        let rest = &self.src[self.pos..];
        if rest.first() == Some(&b'[')
            || (rest.len() >= 5 && rest[..5].eq_ignore_ascii_case(b"array"))
        {
            return self.array();
        }
        let end = if matches!(rest.first(), Some(b'\'') | Some(b'"')) {
            let quote = rest[0];
            match rest[1..].iter().position(|b| *b == quote) {
                Some(inner) => inner + 2,
                None => return false,
            }
        } else {
            rest.iter()
                .position(|b| b.is_ascii_whitespace() || matches!(b, b',' | b')' | b']' | b'='))
                .unwrap_or(rest.len())
        };
        let token = &rest[..end];
        let plain = match token.first() {
            Some(b'\'') | Some(b'"') => token
                .iter()
                .all(|b| b.is_ascii_graphic() && !matches!(b, b'\\' | b'$') || *b == b' '),
            _ => std::str::from_utf8(token).is_ok_and(|t| is_keyword(t) || is_plain_number(t)),
        };
        self.pos += end;
        plain
    }
}

/// Keys that keep their meaning in the output: an integer or a quoted string.
fn is_key(token: &[u8]) -> bool {
    let text = String::from_utf8_lossy(token);
    let text = text.trim();
    let is_integer = !text.contains('.') && is_plain_number(text);
    is_integer || text.starts_with('\'') || text.starts_with('"')
}

#[cfg(test)]
mod tests {
    use super::is_plain_literal_default as plain;

    #[test]
    fn literals_are_plain() {
        for text in [
            "null",
            "NULL",
            "true",
            "False",
            "0",
            "42",
            "-1",
            "1.5",
            "''",
            "'plain'",
            r"'\Foo\Bar'",
            r"'it\'s'",
            r"'\\'",
            "\"plain\"",
            "[]",
            "array()",
            "array ()",
            "['a', 'b']",
            "['a' => 1, 'b' => [true, null],]",
            "array('a' => array(1, 2))",
            "[5 => 'a', 6 => 'b']",
        ] {
            assert!(plain(text), "{text}");
        }
    }

    #[test]
    fn expressions_are_not_plain() {
        for text in [
            "2 * 1024 * 1024",
            "1 + 2",
            "'a' . 'b'",
            "self::X",
            "Foo::BAR",
            "Foo::class",
            "E_ALL",
            "__CLASS__",
            "new Foo()",
            "new \\Foo(1, 2)",
            "010",
            "0x10",
            "0b11",
            "1_000",
            "1e3",
            ".5",
            "\"a\\tb\"",
            "\"$x\"",
            "<<<TEXT\nabc\nTEXT",
            "[self::X]",
            "['a' => 2 * 8]",
            "[Foo::BAR => 1]",
            "['a' . 'b']",
            "[5 => 'a', 'b']",
            "['a' => 1, 'b']",
            r"['Foo\Bar']",
            r"['it\'s']",
            "['é']",
            "[new Foo()]",
            "array_keys([])",
            "99999999999999999999",
        ] {
            assert!(!plain(text), "{text}");
        }
    }
}
