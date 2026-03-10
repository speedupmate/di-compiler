/// Tier 1 custom state-machine PHP lexer.
///
/// Extracts ClassInfo from a PHP file without a full parse tree.
/// Handles ~99% of Magento PHP files. Returns `LexError::Unsupported` for
/// intersection types or unsupported patterns, triggering escalation to Tier 2.
use std::path::Path;

use crate::types::{
    ClassInfo, ClassKind, Constructor, ConstructorParam, LexError, MethodParam, MethodSignature,
};

pub struct Lexer;

impl Lexer {
    pub fn extract(path: &Path) -> Result<ClassInfo, LexError> {
        let bytes = std::fs::read(path)?;
        let mut s = Scanner::new(&bytes);
        s.extract_class(path)
    }
}

struct Scanner<'a> {
    src: &'a [u8],
    pos: usize,
    /// Current file namespace (populated when `namespace` keyword is parsed).
    namespace: String,
    /// File-level `use` imports: short alias → fully-qualified class name.
    use_map: std::collections::HashMap<String, String>,
}

impl<'a> Scanner<'a> {
    fn new(src: &'a [u8]) -> Self {
        Scanner {
            src,
            pos: 0,
            namespace: String::new(),
            use_map: std::collections::HashMap::new(),
        }
    }

    fn at(&self, offset: usize) -> u8 {
        self.src.get(self.pos + offset).copied().unwrap_or(0)
    }

    fn peek(&self) -> u8 {
        self.at(0)
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

    fn skip_heredoc(&mut self) {
        self.skip_ws();
        let nowdoc = self.peek() == b'\'';
        if nowdoc {
            self.advance(1);
        }
        let label_start = self.pos;
        while !self.is_eof() && is_ident_char(self.peek()) {
            self.advance(1);
        }
        let label = self.src[label_start..self.pos].to_vec();
        if nowdoc && self.peek() == b'\'' {
            self.advance(1);
        }
        while !self.is_eof() && self.peek() != b'\n' {
            self.advance(1);
        }
        loop {
            if self.is_eof() {
                return;
            }
            if self.peek() == b'\n' {
                self.advance(1);
            }
            if self.src.len() >= self.pos + label.len() {
                let candidate = &self.src[self.pos..self.pos + label.len()];
                if candidate == label.as_slice() {
                    let after = self.pos + label.len();
                    let next = self.src.get(after).copied().unwrap_or(0);
                    if next == b';' || next == b'\n' || next == b'\r' || next == 0 {
                        self.advance(label.len());
                        if self.peek() == b';' {
                            self.advance(1);
                        }
                        return;
                    }
                }
            }
            while !self.is_eof() && self.peek() != b'\n' {
                self.advance(1);
            }
        }
    }

    fn skip_noise(&mut self) {
        loop {
            self.skip_ws();
            if self.is_eof() {
                return;
            }
            match self.peek() {
                b'/' if self.at(1) == b'/' => {
                    self.advance(2);
                    self.skip_line_comment();
                }
                b'/' if self.at(1) == b'*' => {
                    self.advance(2);
                    self.skip_block_comment();
                }
                b'#' if self.at(1) != b'[' => {
                    self.advance(1);
                    self.skip_line_comment();
                }
                b'\'' => {
                    self.advance(1);
                    self.skip_sq_string();
                }
                b'"' => {
                    self.advance(1);
                    self.skip_dq_string();
                }
                b'<' if self.at(1) == b'<' && self.at(2) == b'<' => {
                    self.advance(3);
                    self.skip_heredoc();
                }
                _ => break,
            }
        }
    }

    fn read_word(&mut self) -> &'a [u8] {
        let start = self.pos;
        while !self.is_eof() && is_word_char(self.peek()) {
            self.advance(1);
        }
        &self.src[start..self.pos]
    }

    fn peek_word_eq(&mut self, keyword: &[u8]) -> bool {
        let saved = self.pos;
        self.skip_noise();
        if !is_word_start(self.peek()) {
            self.pos = saved;
            return false;
        }
        let w = self.read_word();
        let eq = w.eq_ignore_ascii_case(keyword);
        self.pos = saved;
        eq
    }

    fn extract_class(&mut self, path: &Path) -> Result<ClassInfo, LexError> {
        self.find_php_open_tag();

        let mut namespace = String::new();
        let mut class_name: Option<String> = None;
        let mut kind = ClassKind::Class;
        let mut is_abstract = false;
        let mut is_final = false;
        let mut extends: Option<String> = None;
        let mut implements: Vec<String> = Vec::new();

        loop {
            self.skip_noise();
            if self.is_eof() {
                break;
            }
            let b = self.peek();
            if !is_word_start(b) {
                self.advance(1);
                continue;
            }
            let word = self.read_word();
            match word {
                b"namespace" => {
                    namespace = self.read_namespace_decl()?;
                    self.namespace = namespace.clone();
                }
                b"use" => {
                    // File-level use statement (before the class keyword).
                    // Class-level `use TraitName;` cannot appear here since we
                    // haven't entered the class body yet.
                    self.read_use_stmt();
                }
                b"abstract" => {
                    self.skip_noise();
                    if self.peek_word_eq(b"class") {
                        self.read_word();
                        is_abstract = true;
                        kind = ClassKind::AbstractClass;
                        let n = self.read_class_header(&mut extends, &mut implements)?;
                        class_name = Some(n);
                        break;
                    }
                }
                b"final" => {
                    is_final = true;
                    self.skip_noise();
                    if self.peek_word_eq(b"abstract") {
                        self.read_word();
                        self.skip_noise();
                    }
                    if self.peek_word_eq(b"readonly") {
                        self.read_word();
                        self.skip_noise();
                    }
                    if self.peek_word_eq(b"class") {
                        self.read_word();
                        kind = ClassKind::Class;
                        let n = self.read_class_header(&mut extends, &mut implements)?;
                        class_name = Some(n);
                        break;
                    }
                }
                b"readonly" => {
                    self.skip_noise();
                    if self.peek_word_eq(b"class") {
                        self.read_word();
                        kind = ClassKind::Class;
                        let n = self.read_class_header(&mut extends, &mut implements)?;
                        class_name = Some(n);
                        break;
                    }
                }
                b"class" => {
                    kind = ClassKind::Class;
                    let n = self.read_class_header(&mut extends, &mut implements)?;
                    class_name = Some(n);
                    break;
                }
                b"interface" => {
                    kind = ClassKind::Interface;
                    let n = self.read_class_header(&mut extends, &mut implements)?;
                    class_name = Some(n);
                    break;
                }
                b"trait" => {
                    kind = ClassKind::Trait;
                    let n = self.read_class_header(&mut extends, &mut implements)?;
                    class_name = Some(n);
                    break;
                }
                b"enum" => {
                    return Err(LexError::Unsupported("enum".into()));
                }
                _ => {}
            }
        }

        let name = match class_name {
            Some(n) => n,
            None => return Err(LexError::Unsupported("no_class".into())),
        };

        let fqcn = if namespace.is_empty() {
            name.clone()
        } else {
            format!("{}\\{}", namespace, name)
        };

        let (constructor, public_methods) = self.parse_class_body()?;

        Ok(ClassInfo {
            path: path.to_path_buf(),
            namespace,
            name,
            fqcn,
            kind,
            extends,
            implements,
            constructor,
            is_abstract,
            is_final,
            public_methods,
        })
    }

    fn find_php_open_tag(&mut self) {
        while !self.is_eof() {
            if self.peek() == b'<' && self.at(1) == b'?' {
                self.advance(2);
                if self.src[self.pos..].starts_with(b"php") {
                    self.advance(3);
                } else if self.peek() == b'=' {
                    self.advance(1);
                }
                return;
            }
            self.advance(1);
        }
    }

    fn read_namespace_decl(&mut self) -> Result<String, LexError> {
        self.skip_ws();
        let mut ns = String::new();
        while !self.is_eof() {
            let b = self.peek();
            if is_word_char(b) {
                let w = self.read_word();
                ns.push_str(std::str::from_utf8(w).unwrap_or(""));
            } else if b == b'\\' {
                ns.push('\\');
                self.advance(1);
            } else {
                break;
            }
        }
        while !self.is_eof() {
            let b = self.peek();
            if b == b';' || b == b'{' {
                self.advance(1);
                break;
            }
            self.advance(1);
        }
        Ok(ns)
    }

    /// Parse a file-level `use` statement and populate `self.use_map`.
    ///
    /// Handles:
    ///   use Foo\Bar;                       → Bar → Foo\Bar
    ///   use Foo\Bar as Baz;                → Baz → Foo\Bar
    ///   use Foo\Bar\{Baz, Qux as Q};       → Baz → Foo\Bar\Baz, Q → Foo\Bar\Qux
    ///   use function ...; use const ...;   → ignored
    fn read_use_stmt(&mut self) {
        self.skip_noise();
        // Skip `use function` and `use const`
        let saved = self.pos;
        if is_word_start(self.peek()) {
            let w = self.read_word();
            if w == b"function" || w == b"const" {
                self.skip_to_semicolon();
                return;
            }
            self.pos = saved;
        }
        // Skip leading `\`
        if self.peek() == b'\\' {
            self.advance(1);
        }
        // Read base FQCN parts
        let mut base = String::new();
        loop {
            let b = self.peek();
            if is_word_char(b) || b == b'_' {
                let w = self.read_word();
                base.push_str(std::str::from_utf8(w).unwrap_or(""));
            } else if b == b'\\' {
                self.advance(1);
                // Group import: use Foo\Bar\{Baz, Qux as Q};
                if self.peek() == b'{' {
                    self.advance(1);
                    loop {
                        self.skip_noise();
                        if self.is_eof() || self.peek() == b'}' {
                            break;
                        }
                        if self.peek() == b',' {
                            self.advance(1);
                            continue;
                        }
                        // Read one entry in the group
                        let mut part = String::new();
                        loop {
                            let c = self.peek();
                            if is_word_char(c) || c == b'_' {
                                let w = self.read_word();
                                part.push_str(std::str::from_utf8(w).unwrap_or(""));
                            } else if c == b'\\' {
                                part.push('\\');
                                self.advance(1);
                            } else {
                                break;
                            }
                        }
                        // Optional `as Alias`
                        let alias = self.try_read_as_alias().unwrap_or_else(|| {
                            part.split('\\').last().unwrap_or(&part).to_string()
                        });
                        if !alias.is_empty() && !part.is_empty() {
                            self.use_map
                                .insert(alias.to_ascii_lowercase(), format!("{}\\{}", base, part));
                        }
                    }
                    self.skip_noise();
                    if self.peek() == b'}' {
                        self.advance(1);
                    }
                    self.skip_noise();
                    if self.peek() == b';' {
                        self.advance(1);
                    }
                    return;
                }
                base.push('\\');
            } else {
                break;
            }
        }
        if base.is_empty() {
            self.skip_to_semicolon();
            return;
        }
        // Optional `as Alias`
        let alias = self
            .try_read_as_alias()
            .unwrap_or_else(|| base.split('\\').last().unwrap_or(&base).to_string());
        if !alias.is_empty() {
            self.use_map.insert(alias.to_ascii_lowercase(), base);
        }
        self.skip_noise();
        if self.peek() == b';' {
            self.advance(1);
        }
    }

    fn try_read_as_alias(&mut self) -> Option<String> {
        self.skip_noise();
        let saved = self.pos;
        if !is_word_start(self.peek()) {
            return None;
        }
        let w = self.read_word();
        if w == b"as" {
            self.skip_noise();
            if is_word_start(self.peek()) {
                let alias = self.read_word();
                return Some(std::str::from_utf8(alias).unwrap_or("").to_string());
            }
        }
        self.pos = saved;
        None
    }

    fn skip_to_semicolon(&mut self) {
        while !self.is_eof() && self.peek() != b';' {
            self.advance(1);
        }
        if self.peek() == b';' {
            self.advance(1);
        }
    }

    /// Resolve a bare (non-absolute) PHP type hint to a FQCN using
    /// the current namespace and file-level use imports.
    fn resolve_type(&self, raw: &str) -> String {
        // Primitives stay as-is — they are never FQCNs.
        if is_primitive_type(raw) {
            return raw.to_string();
        }
        // The first segment determines whether there's a use-import match.
        let first = raw.split('\\').next().unwrap_or(raw);
        if let Some(mapped) = self.use_map.get(&first.to_ascii_lowercase()) {
            if raw.contains('\\') {
                // e.g. `use Foo\Bar;` + type `Bar\Baz` → `Foo\Bar\Baz`
                let rest = &raw[first.len() + 1..];
                return format!("{}\\{}", mapped, rest);
            }
            return mapped.clone();
        }
        // Relative to current namespace.
        if self.namespace.is_empty() {
            raw.to_string()
        } else {
            format!("{}\\{}", self.namespace, raw)
        }
    }

    fn read_class_header(
        &mut self,
        extends: &mut Option<String>,
        implements: &mut Vec<String>,
    ) -> Result<String, LexError> {
        self.skip_noise();
        if !is_word_start(self.peek()) {
            return Err(LexError::Unsupported("anonymous_class".into()));
        }
        let name_bytes = self.read_word();
        let name = std::str::from_utf8(name_bytes).unwrap_or("").to_string();

        loop {
            self.skip_noise();
            if self.is_eof() {
                return Err(LexError::UnexpectedEof);
            }
            let b = self.peek();
            if b == b'{' {
                self.advance(1);
                break;
            }
            if !is_word_start(b) {
                self.advance(1);
                continue;
            }
            let word = self.read_word();
            match word {
                b"extends" => {
                    let fqn = self.read_fqn()?;
                    if extends.is_none() {
                        *extends = Some(fqn);
                    } else if !fqn.is_empty() {
                        implements.push(fqn);
                    }
                    loop {
                        self.skip_noise();
                        if self.peek() == b',' {
                            self.advance(1);
                            let extra = self.read_fqn()?;
                            if !extra.is_empty() {
                                implements.push(extra);
                            }
                        } else {
                            break;
                        }
                    }
                }
                b"implements" => {
                    self.read_fqn_list(implements)?;
                }
                _ => {}
            }
        }

        Ok(name)
    }

    fn read_fqn(&mut self) -> Result<String, LexError> {
        self.skip_noise();
        let absolute = self.peek() == b'\\';
        if absolute {
            self.advance(1);
        }
        let mut fqn = String::new();
        loop {
            let b = self.peek();
            if is_word_char(b) {
                let w = self.read_word();
                fqn.push_str(std::str::from_utf8(w).unwrap_or(""));
            } else if b == b'\\' {
                fqn.push('\\');
                self.advance(1);
            } else {
                break;
            }
        }
        if !absolute && !fqn.is_empty() {
            fqn = self.resolve_type(&fqn);
        }
        Ok(fqn)
    }

    fn read_fqn_list(&mut self, list: &mut Vec<String>) -> Result<(), LexError> {
        loop {
            let fqn = self.read_fqn()?;
            if !fqn.is_empty() {
                list.push(fqn);
            }
            self.skip_noise();
            if self.peek() == b',' {
                self.advance(1);
            } else {
                break;
            }
        }
        Ok(())
    }

    fn parse_class_body(
        &mut self,
    ) -> Result<(Option<Constructor>, Vec<MethodSignature>), LexError> {
        let mut constructor: Option<Constructor> = None;
        let mut public_methods: Vec<MethodSignature> = Vec::new();
        let mut depth: u32 = 1;

        loop {
            if self.is_eof() || depth == 0 {
                break;
            }

            if depth != 1 {
                match self.peek() {
                    b'\'' => {
                        self.advance(1);
                        self.skip_sq_string();
                    }
                    b'"' => {
                        self.advance(1);
                        self.skip_dq_string();
                    }
                    b'/' if self.at(1) == b'/' => {
                        self.advance(2);
                        self.skip_line_comment();
                    }
                    b'/' if self.at(1) == b'*' => {
                        self.advance(2);
                        self.skip_block_comment();
                    }
                    b'#' if self.at(1) != b'[' => {
                        self.advance(1);
                        self.skip_line_comment();
                    }
                    b'<' if self.at(1) == b'<' && self.at(2) == b'<' => {
                        self.advance(3);
                        self.skip_heredoc();
                    }
                    b'{' => {
                        depth += 1;
                        self.advance(1);
                    }
                    b'}' => {
                        depth -= 1;
                        self.advance(1);
                    }
                    _ => self.advance(1),
                }
                continue;
            }

            // depth == 1
            self.skip_noise();
            if self.is_eof() {
                break;
            }

            let b = self.peek();
            if b == b'{' {
                depth += 1;
                self.advance(1);
                continue;
            }
            if b == b'}' {
                depth -= 1;
                self.advance(1);
                continue;
            }
            if b == b'#' && self.at(1) == b'[' {
                self.skip_php_attribute();
                continue;
            }

            if !is_word_start(b) {
                self.advance(1);
                continue;
            }

            let mut is_abstract_method = false;
            let mut is_final_method = false;
            let mut is_public = false;
            let mut is_static = false;
            let mut saw_function = false;

            let first_word = self.read_word();
            let is_modifier = matches!(
                first_word,
                b"abstract"
                    | b"final"
                    | b"public"
                    | b"private"
                    | b"protected"
                    | b"static"
                    | b"readonly"
                    | b"function"
            );
            if !is_modifier {
                self.skip_to_stmt_end(&mut depth);
                continue;
            }

            let mut current = first_word;
            loop {
                match current {
                    b"abstract" => is_abstract_method = true,
                    b"final" => is_final_method = true,
                    b"public" => is_public = true,
                    b"private" | b"protected" => {}
                    b"static" => is_static = true,
                    b"readonly" => {}
                    b"function" => {
                        saw_function = true;
                        break;
                    }
                    _ => break,
                }
                self.skip_noise();
                if is_word_start(self.peek()) {
                    current = self.read_word();
                } else {
                    break;
                }
            }

            if !saw_function {
                self.skip_to_stmt_end(&mut depth);
                continue;
            }

            let mut returns_reference = false;
            self.skip_noise();
            if self.peek() == b'&' {
                returns_reference = true;
                self.advance(1);
                self.skip_noise();
            }
            if !is_word_start(self.peek()) {
                self.skip_to_stmt_end(&mut depth);
                continue;
            }
            let method_name_bytes = self.read_word();
            let method_name = std::str::from_utf8(method_name_bytes)
                .unwrap_or("")
                .to_string();

            self.skip_noise();
            if self.peek() != b'(' {
                self.skip_to_stmt_end(&mut depth);
                continue;
            }
            self.advance(1);

            if method_name == "__construct" {
                let params = self.parse_param_list()?;
                constructor = Some(Constructor { params });
                self.skip_noise();
                if self.peek() == b':' {
                    self.advance(1);
                    self.skip_noise();
                    self.skip_type_hint()?;
                }
                self.skip_method_body_braces(&mut depth);
            } else if is_public && !is_final_method {
                let params = self.parse_method_params()?;
                self.skip_noise();
                let return_type = if self.peek() == b':' {
                    self.advance(1);
                    self.skip_noise();
                    Some(self.read_return_type()?)
                } else {
                    None
                };
                self.skip_method_body_braces(&mut depth);
                let _ = is_abstract_method;
                public_methods.push(MethodSignature {
                    name: method_name,
                    params,
                    return_type,
                    is_static,
                    returns_reference,
                });
            } else {
                self.skip_to_matching_paren();
                self.skip_noise();
                if self.peek() == b':' {
                    self.advance(1);
                    self.skip_noise();
                    self.skip_type_hint()?;
                }
                self.skip_method_body_braces(&mut depth);
            }
        }

        Ok((constructor, public_methods))
    }

    fn skip_to_stmt_end(&mut self, depth: &mut u32) {
        loop {
            self.skip_noise();
            if self.is_eof() {
                break;
            }
            match self.peek() {
                b';' => {
                    self.advance(1);
                    break;
                }
                b'{' => {
                    *depth += 1;
                    self.advance(1);
                    break;
                }
                b'}' => {
                    if *depth > 0 {
                        *depth -= 1;
                    }
                    self.advance(1);
                    break;
                }
                _ => self.advance(1),
            }
        }
    }

    fn parse_param_list(&mut self) -> Result<Vec<ConstructorParam>, LexError> {
        let mut params = Vec::new();
        loop {
            self.skip_noise();
            if self.is_eof() {
                break;
            }
            match self.peek() {
                b')' => {
                    self.advance(1);
                    break;
                }
                b',' => {
                    self.advance(1);
                    continue;
                }
                b'#' if self.at(1) == b'[' => {
                    self.skip_php_attribute();
                    continue;
                }
                _ => {}
            }
            if let Some(p) = self.parse_one_ctor_param()? {
                params.push(p);
            }
        }
        Ok(params)
    }

    fn parse_method_params(&mut self) -> Result<Vec<MethodParam>, LexError> {
        let mut params = Vec::new();
        loop {
            self.skip_noise();
            if self.is_eof() {
                break;
            }
            match self.peek() {
                b')' => {
                    self.advance(1);
                    break;
                }
                b',' => {
                    self.advance(1);
                    continue;
                }
                b'#' if self.at(1) == b'[' => {
                    self.skip_php_attribute();
                    continue;
                }
                _ => {}
            }
            if let Some(p) = self.parse_one_method_param()? {
                params.push(p);
            }
        }
        Ok(params)
    }

    fn parse_one_ctor_param(&mut self) -> Result<Option<ConstructorParam>, LexError> {
        self.skip_noise();
        let mut is_promoted = false;
        let mut is_readonly = false;

        if is_word_start(self.peek()) {
            let saved = self.pos;
            let word = self.read_word();
            match word {
                b"public" | b"private" | b"protected" => {
                    is_promoted = true;
                    self.skip_noise();
                    if self.peek_word_eq(b"readonly") {
                        self.read_word();
                        is_readonly = true;
                    }
                }
                word if word.eq_ignore_ascii_case(b"readonly") => {
                    is_readonly = true;
                }
                _ => self.pos = saved,
            }
        }
        let _ = is_readonly;

        let type_hint = self.try_read_type_hint()?;
        let is_nullable = type_hint
            .as_deref()
            .map(|t| t.starts_with('?'))
            .unwrap_or(false);

        self.skip_noise();
        if type_hint.is_some() && self.peek() == b'&' && self.at(1) != b'$' && self.at(1) != b'&' {
            return Err(LexError::Unsupported("intersection_type".into()));
        }

        let mut is_variadic = false;
        if self.peek() == b'.' && self.at(1) == b'.' && self.at(2) == b'.' {
            is_variadic = true;
            self.advance(3);
        }
        if self.peek() == b'&' {
            self.advance(1);
        }

        self.skip_noise();
        if self.peek() != b'$' {
            self.skip_to_param_end();
            return Ok(None);
        }
        self.advance(1);
        let name_bytes = self.read_word();
        let name = std::str::from_utf8(name_bytes).unwrap_or("").to_string();

        self.skip_noise();
        let mut is_optional = false;
        let mut default_value = None;
        if self.peek() == b'=' {
            is_optional = true;
            self.advance(1);
            let value = self.read_default_value();
            if !value.is_empty() {
                default_value = Some(value);
            }
        }

        let is_primitive = type_hint.as_deref().map(is_primitive_type).unwrap_or(true);

        Ok(Some(ConstructorParam {
            name,
            type_hint,
            is_optional,
            default_value,
            is_primitive,
            is_variadic,
            is_promoted,
        }))
    }

    fn parse_one_method_param(&mut self) -> Result<Option<MethodParam>, LexError> {
        self.skip_noise();
        if is_word_start(self.peek()) {
            let saved = self.pos;
            let word = self.read_word();
            match word {
                b"public" | b"private" | b"protected" | b"readonly" => {}
                _ => self.pos = saved,
            }
        }
        let type_hint = self.try_read_type_hint()?;

        self.skip_noise();
        if type_hint.is_some() && self.peek() == b'&' && self.at(1) != b'$' && self.at(1) != b'&' {
            return Err(LexError::Unsupported("intersection_type".into()));
        }

        let mut is_by_ref = false;
        self.skip_noise();
        if self.peek() == b'&' {
            is_by_ref = true;
            self.advance(1);
        }

        let mut is_variadic = false;
        if self.peek() == b'.' && self.at(1) == b'.' && self.at(2) == b'.' {
            is_variadic = true;
            self.advance(3);
        }
        if self.peek() == b'&' {
            is_by_ref = true;
            self.advance(1);
        }

        self.skip_noise();
        if self.peek() != b'$' {
            self.skip_to_param_end();
            return Ok(None);
        }
        self.advance(1);
        let name_bytes = self.read_word();
        let name = std::str::from_utf8(name_bytes).unwrap_or("").to_string();

        self.skip_noise();
        let mut has_default = false;
        let mut default_value = None;
        if self.peek() == b'=' {
            has_default = true;
            self.advance(1);
            let value = self.read_default_value();
            if !value.is_empty() {
                default_value = Some(value);
            }
        }

        Ok(Some(MethodParam {
            name,
            type_hint,
            has_default,
            default_value,
            is_variadic,
            is_by_ref,
        }))
    }

    fn try_read_type_hint(&mut self) -> Result<Option<String>, LexError> {
        self.skip_noise();
        let mut is_nullable = false;
        if self.peek() == b'?' {
            is_nullable = true;
            self.advance(1);
            self.skip_noise();
        }

        let b = self.peek();
        if b == b'$' || b == b')' || b == b',' || b == b'.' || b == b'&' {
            return Ok(None);
        }
        if !is_word_start(b) && b != b'\\' {
            return Ok(None);
        }
        let absolute = self.peek() == b'\\';
        if absolute {
            self.advance(1);
        }
        let start = self.pos;
        while !self.is_eof() && (is_word_char(self.peek()) || self.peek() == b'\\') {
            self.advance(1);
        }
        let raw_type = std::str::from_utf8(&self.src[start..self.pos])
            .unwrap_or("")
            .to_string();
        if raw_type.is_empty() {
            return Ok(None);
        }

        let first = if absolute {
            raw_type.clone()
        } else {
            self.resolve_type(&raw_type)
        };
        let mut parts: Vec<String> = vec![first];

        while self.peek() == b'|' {
            self.advance(1);
            let part_abs = self.peek() == b'\\';
            if part_abs {
                self.advance(1);
            }
            let pstart = self.pos;
            while !self.is_eof() && (is_word_char(self.peek()) || self.peek() == b'\\') {
                self.advance(1);
            }
            let part = std::str::from_utf8(&self.src[pstart..self.pos])
                .unwrap_or("")
                .to_string();
            if !part.is_empty() {
                let resolved = if part_abs {
                    part
                } else {
                    self.resolve_type(&part)
                };
                parts.push(resolved);
            }
        }

        let mut resolved = if parts.len() > 1 {
            parts.join("|")
        } else {
            parts.remove(0)
        };
        if is_nullable {
            resolved = format!("?{resolved}");
        }
        Ok(Some(resolved))
    }

    fn read_return_type(&mut self) -> Result<String, LexError> {
        match self.try_read_type_hint()? {
            Some(t) => Ok(t),
            None => Ok(String::new()),
        }
    }

    fn skip_type_hint(&mut self) -> Result<(), LexError> {
        self.try_read_type_hint()?;
        Ok(())
    }

    fn read_default_value(&mut self) -> String {
        let start = self.pos;
        let mut paren: i32 = 0;
        let mut bracket: i32 = 0;
        loop {
            if self.is_eof() {
                break;
            }
            match self.peek() {
                b'\'' => {
                    self.advance(1);
                    self.skip_sq_string();
                }
                b'"' => {
                    self.advance(1);
                    self.skip_dq_string();
                }
                b'/' if self.at(1) == b'/' => {
                    // Line comment ends the meaningful part of the default value.
                    // Break here leaving pos at `//`; the outer loop's skip_noise handles it.
                    break;
                }
                b'/' if self.at(1) == b'*' => {
                    self.advance(2);
                    self.skip_block_comment();
                }
                b'#' if self.at(1) != b'[' => {
                    // Line comment — same treatment as `//`.
                    break;
                }
                b'(' => {
                    paren += 1;
                    self.advance(1);
                }
                b')' if paren == 0 => break,
                b')' => {
                    paren -= 1;
                    self.advance(1);
                }
                b'[' => {
                    bracket += 1;
                    self.advance(1);
                }
                b']' if bracket > 0 => {
                    bracket -= 1;
                    self.advance(1);
                }
                b',' if paren == 0 && bracket == 0 => break,
                _ => self.advance(1),
            }
        }
        std::str::from_utf8(&self.src[start..self.pos])
            .unwrap_or("")
            .trim()
            .to_string()
    }

    fn skip_to_param_end(&mut self) {
        let mut depth: i32 = 0;
        loop {
            if self.is_eof() {
                break;
            }
            match self.peek() {
                b'\'' => {
                    self.advance(1);
                    self.skip_sq_string();
                }
                b'"' => {
                    self.advance(1);
                    self.skip_dq_string();
                }
                b'(' | b'[' => {
                    depth += 1;
                    self.advance(1);
                }
                b')' if depth == 0 => break,
                b')' => {
                    depth -= 1;
                    self.advance(1);
                }
                b']' if depth > 0 => {
                    depth -= 1;
                    self.advance(1);
                }
                b',' if depth == 0 => break,
                _ => self.advance(1),
            }
        }
    }

    fn skip_to_matching_paren(&mut self) {
        let mut depth: i32 = 1;
        loop {
            self.skip_noise();
            if self.is_eof() || depth == 0 {
                break;
            }
            match self.peek() {
                b'(' => {
                    depth += 1;
                    self.advance(1);
                }
                b')' => {
                    depth -= 1;
                    self.advance(1);
                }
                _ => self.advance(1),
            }
        }
    }

    fn skip_method_body_braces(&mut self, _class_depth: &mut u32) {
        self.skip_noise();
        if self.peek() == b';' {
            self.advance(1);
            return;
        }
        if self.peek() != b'{' {
            return;
        }
        self.advance(1);
        let mut depth: u32 = 1;
        loop {
            if self.is_eof() || depth == 0 {
                break;
            }
            match self.peek() {
                b'\'' => {
                    self.advance(1);
                    self.skip_sq_string();
                }
                b'"' => {
                    self.advance(1);
                    self.skip_dq_string();
                }
                b'/' if self.at(1) == b'/' => {
                    self.advance(2);
                    self.skip_line_comment();
                }
                b'/' if self.at(1) == b'*' => {
                    self.advance(2);
                    self.skip_block_comment();
                }
                b'#' if self.at(1) != b'[' => {
                    self.advance(1);
                    self.skip_line_comment();
                }
                b'<' if self.at(1) == b'<' && self.at(2) == b'<' => {
                    self.advance(3);
                    self.skip_heredoc();
                }
                b'{' => {
                    depth += 1;
                    self.advance(1);
                }
                b'}' => {
                    depth -= 1;
                    self.advance(1);
                }
                _ => self.advance(1),
            }
        }
    }

    fn skip_php_attribute(&mut self) {
        self.advance(1); // `#`
        if self.peek() == b'[' {
            self.advance(1);
            let mut depth = 1u32;
            loop {
                if self.is_eof() || depth == 0 {
                    break;
                }
                match self.peek() {
                    b'[' => {
                        depth += 1;
                        self.advance(1);
                    }
                    b']' => {
                        depth -= 1;
                        self.advance(1);
                    }
                    _ => self.advance(1),
                }
            }
        }
    }
}

fn is_word_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_word_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn is_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn is_primitive_type(t: &str) -> bool {
    let t = t.trim_start_matches('?');
    if t.contains('|') {
        return t.split('|').all(is_primitive_base);
    }
    is_primitive_base(t)
}

fn is_primitive_base(t: &str) -> bool {
    matches!(
        t,
        "int"
            | "integer"
            | "float"
            | "double"
            | "string"
            | "bool"
            | "boolean"
            | "array"
            | "callable"
            | "iterable"
            | "void"
            | "null"
            | "mixed"
            | "never"
            | "true"
            | "false"
            | "object"
            | "self"
            | "static"
            | "parent"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn extract_str(php: &str) -> Result<ClassInfo, LexError> {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(php.as_bytes()).unwrap();
        Lexer::extract(f.path())
    }

    #[test]
    fn test_simple_class() {
        let info = extract_str("<?php\nnamespace Foo\\Bar;\nclass Baz {}").unwrap();
        assert_eq!(info.namespace, "Foo\\Bar");
        assert_eq!(info.name, "Baz");
        assert_eq!(info.fqcn, "Foo\\Bar\\Baz");
        assert!(matches!(info.kind, ClassKind::Class));
    }

    #[test]
    fn test_interface() {
        let info = extract_str("<?php\nnamespace Foo;\ninterface MyIface {}").unwrap();
        assert!(matches!(info.kind, ClassKind::Interface));
    }

    #[test]
    fn test_abstract_class() {
        let info = extract_str("<?php\nnamespace Foo;\nabstract class AbstractThing {}").unwrap();
        assert!(info.is_abstract);
        assert!(matches!(info.kind, ClassKind::AbstractClass));
    }

    #[test]
    fn test_final_class() {
        let info = extract_str("<?php\nnamespace Foo;\nfinal class Thing {}").unwrap();
        assert!(info.is_final);
    }

    #[test]
    fn test_extends_implements() {
        let info = extract_str(
            "<?php\nnamespace Foo;\nclass Bar extends \\Base\\Foo implements \\My\\Iface, Other {}",
        )
        .unwrap();
        assert_eq!(info.extends.as_deref(), Some("Base\\Foo"));
        // `Other` has no `use` import and no leading `\`, so it resolves to `Foo\Other`
        assert_eq!(info.implements, vec!["My\\Iface", "Foo\\Other"]);
    }

    #[test]
    fn test_constructor_params() {
        let info = extract_str(
            r#"<?php
namespace Foo;
class Bar {
    public function __construct(
        \Magento\Framework\Thing $thing,
        ?string $name = null,
        array $items = []
    ) {}
}"#,
        )
        .unwrap();
        let ctor = info.constructor.unwrap();
        assert_eq!(ctor.params.len(), 3);
        assert_eq!(
            ctor.params[0].type_hint.as_deref(),
            Some("Magento\\Framework\\Thing")
        );
        assert!(!ctor.params[0].is_optional);
        assert_eq!(ctor.params[1].type_hint.as_deref(), Some("?string"));
        assert!(ctor.params[1].is_optional);
        assert!(ctor.params[2].is_primitive);
    }

    #[test]
    fn test_enum_returns_error() {
        let result = extract_str("<?php\nenum Status { case Active; }");
        assert!(matches!(result, Err(LexError::Unsupported(_))));
    }

    #[test]
    fn test_intersection_type_escalates() {
        let result = extract_str(
            "<?php\nnamespace Foo;\nclass Bar {\n    public function __construct(Foo&Bar $x) {}\n}",
        );
        assert!(matches!(result, Err(LexError::Unsupported(_))));
    }

    #[test]
    fn test_constructor_promotion() {
        let info = extract_str(
            r#"<?php
namespace Foo;
class Bar {
    public function __construct(
        public readonly \Foo\Service $service,
        private string $name = 'default'
    ) {}
}"#,
        )
        .unwrap();
        let ctor = info.constructor.unwrap();
        assert_eq!(ctor.params.len(), 2);
        assert!(ctor.params[0].is_promoted);
        assert!(ctor.params[1].is_promoted);
    }

    #[test]
    fn test_public_methods() {
        let info = extract_str(
            r#"<?php
namespace Foo;
class Bar {
    public function doThing(string $x): string { return $x; }
    private function secretMethod(): void {}
    final public function finalMethod(): void {}
    public static function staticMethod(): void {}
}"#,
        )
        .unwrap();
        assert_eq!(info.public_methods.len(), 2);
        let names: Vec<&str> = info
            .public_methods
            .iter()
            .map(|m| m.name.as_str())
            .collect();
        assert!(names.contains(&"doThing"));
        assert!(names.contains(&"staticMethod"));
    }

    #[test]
    fn test_no_namespace() {
        let info = extract_str("<?php\nclass Foo {}").unwrap();
        assert_eq!(info.namespace, "");
        assert_eq!(info.fqcn, "Foo");
    }

    #[test]
    fn test_trait() {
        let info = extract_str("<?php\nnamespace Foo;\ntrait MyTrait {}").unwrap();
        assert!(matches!(info.kind, ClassKind::Trait));
    }

    #[test]
    fn test_union_type_preserved() {
        let info = extract_str(
            r#"<?php
namespace Foo;
class Bar {
    public function __construct(Baz|null $dep = null) {}
}"#,
        )
        .unwrap();
        let ctor = info.constructor.unwrap();
        // `Baz` in namespace `Foo` resolves to `Foo\Baz`, union with `null` preserved
        assert_eq!(ctor.params[0].type_hint.as_deref(), Some("Foo\\Baz|null"));
        assert!(ctor.params[0].is_optional);
    }

    #[test]
    fn test_method_nullable_union_and_reference_preserved() {
        let info = extract_str(
            r#"<?php
namespace Foo;
class Bar {
    public function & resolve(\A\B|C|null &$value, D|E $next = null): \X\Y|Z|null
    {
        return $value;
    }
}"#,
        )
        .unwrap();
        assert_eq!(info.public_methods.len(), 1);
        let m = &info.public_methods[0];
        assert_eq!(m.name, "resolve");
        assert!(m.returns_reference);
        assert_eq!(m.params.len(), 2);
        assert_eq!(m.params[0].type_hint.as_deref(), Some("A\\B|Foo\\C|null"));
        assert!(m.params[0].is_by_ref);
        assert_eq!(m.params[1].type_hint.as_deref(), Some("Foo\\D|Foo\\E"));
        assert!(m.params[1].has_default);
        assert_eq!(m.return_type.as_deref(), Some("X\\Y|Foo\\Z|null"));
    }

    #[test]
    fn test_variadic_param() {
        let info = extract_str(
            r#"<?php
namespace Foo;
class Bar {
    public function __construct(\Foo\Dep ...$deps) {}
}"#,
        )
        .unwrap();
        let ctor = info.constructor.unwrap();
        assert!(ctor.params[0].is_variadic);
    }
}
