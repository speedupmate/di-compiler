//! Minimal PHP class-constant extractor.
//!
//! Scans a PHP source file for string-literal class constants:
//!
//!   `const CONST_NAME = 'value';`
//!   `const CONST_NAME = "value";`
//!
//! Used to resolve `xsi:type="init_parameter"` references in di.xml that
//! contain PHP constant expressions like `ClassName::CONST_NAME`.

use rustc_hash::FxHashMap;
use std::path::Path;

/// Extract all string-literal class constants from a PHP source file.
/// Returns a map of `CONST_NAME → resolved_string_value`.
pub fn extract_string_constants(path: &Path) -> FxHashMap<String, String> {
    let Ok(bytes) = std::fs::read(path) else {
        return FxHashMap::default();
    };
    let mut result = FxHashMap::default();
    let mut s = ConstScanner::new(&bytes);
    s.scan(&mut result);
    result
}

struct ConstScanner<'a> {
    src: &'a [u8],
    pos: usize,
}

impl<'a> ConstScanner<'a> {
    fn new(src: &'a [u8]) -> Self {
        ConstScanner { src, pos: 0 }
    }

    fn peek(&self) -> u8 {
        self.src.get(self.pos).copied().unwrap_or(0)
    }

    fn at(&self, offset: usize) -> u8 {
        self.src.get(self.pos + offset).copied().unwrap_or(0)
    }

    fn advance(&mut self, n: usize) {
        self.pos = (self.pos + n).min(self.src.len());
    }

    fn is_eof(&self) -> bool {
        self.pos >= self.src.len()
    }

    fn skip_ws(&mut self) {
        while !self.is_eof() {
            match self.peek() {
                b' ' | b'\t' | b'\n' | b'\r' => self.advance(1),
                _ => break,
            }
        }
    }

    fn skip_line_comment(&mut self) {
        while !self.is_eof() && self.peek() != b'\n' {
            self.advance(1);
        }
    }

    fn skip_block_comment(&mut self) {
        while !self.is_eof() {
            if self.peek() == b'*' && self.at(1) == b'/' {
                self.advance(2);
                return;
            }
            self.advance(1);
        }
    }

    fn skip_sq_string(&mut self) {
        while !self.is_eof() {
            match self.peek() {
                b'\\' => self.advance(2),
                b'\'' => {
                    self.advance(1);
                    return;
                }
                _ => self.advance(1),
            }
        }
    }

    fn skip_dq_string(&mut self) {
        while !self.is_eof() {
            match self.peek() {
                b'\\' => self.advance(2),
                b'"' => {
                    self.advance(1);
                    return;
                }
                _ => self.advance(1),
            }
        }
    }

    fn read_ident(&mut self) -> &'a [u8] {
        let start = self.pos;
        while !self.is_eof() {
            let b = self.peek();
            if b.is_ascii_alphanumeric() || b == b'_' {
                self.advance(1);
            } else {
                break;
            }
        }
        &self.src[start..self.pos]
    }

    /// Read a single-quoted PHP string value (without the surrounding quotes).
    fn read_sq_string(&mut self) -> Option<String> {
        // pos is already past the opening '
        let mut out = Vec::new();
        loop {
            if self.is_eof() {
                return None;
            }
            match self.peek() {
                b'\\' => {
                    self.advance(1);
                    match self.peek() {
                        b'\'' => {
                            out.push(b'\'');
                            self.advance(1);
                        }
                        b'\\' => {
                            out.push(b'\\');
                            self.advance(1);
                        }
                        other => {
                            out.push(b'\\');
                            out.push(other);
                            self.advance(1);
                        }
                    }
                }
                b'\'' => {
                    self.advance(1);
                    break;
                }
                b => {
                    out.push(b);
                    self.advance(1);
                }
            }
        }
        String::from_utf8(out).ok()
    }

    /// Read a double-quoted PHP string value (without the surrounding quotes).
    /// Only handles simple strings without interpolation.
    fn read_dq_string(&mut self) -> Option<String> {
        let mut out = Vec::new();
        loop {
            if self.is_eof() {
                return None;
            }
            match self.peek() {
                b'\\' => {
                    self.advance(1);
                    match self.peek() {
                        b'"' => {
                            out.push(b'"');
                            self.advance(1);
                        }
                        b'\\' => {
                            out.push(b'\\');
                            self.advance(1);
                        }
                        b'n' => {
                            out.push(b'\n');
                            self.advance(1);
                        }
                        b't' => {
                            out.push(b'\t');
                            self.advance(1);
                        }
                        b'r' => {
                            out.push(b'\r');
                            self.advance(1);
                        }
                        other => {
                            out.push(b'\\');
                            out.push(other);
                            self.advance(1);
                        }
                    }
                }
                b'$' | b'{' => {
                    // Interpolated string constants are unsupported; fast-forward
                    // to the closing quote so scanning can continue.
                    while !self.is_eof() {
                        match self.peek() {
                            b'\\' => self.advance(2),
                            b'"' => {
                                self.advance(1);
                                break;
                            }
                            _ => self.advance(1),
                        }
                    }
                    return None;
                }
                b'"' => {
                    self.advance(1);
                    break;
                }
                b => {
                    out.push(b);
                    self.advance(1);
                }
            }
        }
        String::from_utf8(out).ok()
    }

    fn scan(&mut self, result: &mut FxHashMap<String, String>) {
        while !self.is_eof() {
            match self.peek() {
                b'\'' => {
                    self.advance(1);
                    self.skip_sq_string();
                }
                b'"' => {
                    self.advance(1);
                    self.skip_dq_string();
                }
                b'/' => {
                    if self.at(1) == b'/' {
                        self.advance(2);
                        self.skip_line_comment();
                    } else if self.at(1) == b'*' {
                        self.advance(2);
                        self.skip_block_comment();
                    } else {
                        self.advance(1);
                    }
                }
                b'#' => {
                    self.advance(1);
                    self.skip_line_comment();
                }
                b if b.is_ascii_alphabetic() || b == b'_' => {
                    let start = self.pos;
                    let ident = self.read_ident();
                    if ident == b"const" {
                        self.try_read_const(start, result);
                    }
                }
                _ => self.advance(1),
            }
        }
    }

    fn try_read_const(&mut self, _start: usize, result: &mut FxHashMap<String, String>) {
        // Skip whitespace after `const`
        self.skip_ws();
        // Read constant name
        if !self.peek().is_ascii_alphabetic() && self.peek() != b'_' {
            return;
        }
        let name_bytes = self.read_ident();
        let const_name = match std::str::from_utf8(name_bytes) {
            Ok(s) => s.to_string(),
            Err(_) => return,
        };
        self.skip_ws();
        if self.peek() != b'=' {
            return;
        }
        self.advance(1); // consume '='
        self.skip_ws();
        let value = match self.peek() {
            b'\'' => {
                self.advance(1);
                self.read_sq_string()
            }
            b'"' => {
                self.advance(1);
                self.read_dq_string()
            }
            _ => None,
        };
        if let Some(v) = value {
            result.insert(const_name, v);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(src: &str) -> FxHashMap<String, String> {
        let mut out = FxHashMap::default();
        let mut scanner = ConstScanner::new(src.as_bytes());
        scanner.scan(&mut out);
        out
    }

    #[test]
    fn extracts_single_and_double_quoted_constants() {
        let src = r#"
            <?php
            class C {
                const FOO = 'bar';
                const BAZ = "qux";
            }
        "#;
        let out = scan(src);
        assert_eq!(out.get("FOO").map(String::as_str), Some("bar"));
        assert_eq!(out.get("BAZ").map(String::as_str), Some("qux"));
    }

    #[test]
    fn ignores_non_string_constants_and_comments() {
        let src = r#"
            <?php
            // const COMMENTED = 'x';
            /* const BLOCK = "y"; */
            class C {
                const NUM = 42;
                const BOOL = true;
                const ARR = [];
                const OK = 'good';
            }
        "#;
        let out = scan(src);
        assert_eq!(out.len(), 1);
        assert_eq!(out.get("OK").map(String::as_str), Some("good"));
    }

    #[test]
    fn handles_escaped_quotes() {
        let src = r#"
            <?php
            class C {
                const A = 'it\'s';
                const B = "a\"b";
            }
        "#;
        let out = scan(src);
        assert_eq!(out.get("A").map(String::as_str), Some("it's"));
        assert_eq!(out.get("B").map(String::as_str), Some("a\"b"));
    }

    #[test]
    fn skips_interpolated_double_quoted_constants() {
        let src = r#"
            <?php
            class C {
                const BAD1 = "foo $bar";
                const BAD2 = "{$bar}";
                const GOOD = "plain";
            }
        "#;
        let out = scan(src);
        assert_eq!(out.get("GOOD").map(String::as_str), Some("plain"));
        assert!(!out.contains_key("BAD1"));
        assert!(!out.contains_key("BAD2"));
    }

    #[test]
    fn missing_file_returns_empty_map() {
        let out = extract_string_constants(Path::new("/tmp/does-not-exist-constants.php"));
        assert!(out.is_empty());
    }
}
