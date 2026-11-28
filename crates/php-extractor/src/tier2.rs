//! Tier 2: tree-sitter-php fallback extractor.
//!
//! Used when the Tier 1 custom lexer encounters syntax it cannot handle (e.g. intersection types).
use std::path::Path;

use tree_sitter::{Node, Parser};

use crate::lexer::ScanFacts;
use crate::types::{
    ClassInfo, ClassKind, Constructor, ConstructorParam, ExtractResult, MethodParam,
    MethodSignature, TraitData, TraitMethod,
};

pub fn extract_tier2(path: &Path) -> ExtractResult {
    extract_tier2_with_facts(path).0
}

/// Like [`extract_tier2`], and also says whether the declaration is inside a block. A file
/// whose tree has a syntax error is a `ParseFailure`, so it goes on to Tier 3: tree-sitter-php
/// returns a partial tree for syntax it does not know, such as property hooks.
pub fn extract_tier2_with_facts(path: &Path) -> (ExtractResult, ScanFacts) {
    let source = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            return (
                ExtractResult::PhpFallbackFailed(format!("io: {e}")),
                ScanFacts::default(),
            )
        }
    };

    match extract_with_treesitter(&source, path) {
        Ok(Some((info, inside_block))) => (
            ExtractResult::Ok(info),
            ScanFacts {
                class_check: false,
                inside_block,
            },
        ),
        Ok(None) => (ExtractResult::NoClass, ScanFacts::default()),
        Err(e) => (ExtractResult::ParseFailure(e), ScanFacts::default()),
    }
}

/// Whether `node` is inside a block. The file, a namespace and the body of a braced namespace
/// are no blocks, as for Magento's `FileClassScanner`.
fn is_inside_block(node: Node) -> bool {
    let mut current = node;
    while let Some(parent) = current.parent() {
        let top_level = match parent.kind() {
            "program" | "namespace_definition" => true,
            "compound_statement" => parent
                .parent()
                .is_some_and(|above| above.kind() == "namespace_definition"),
            _ => false,
        };
        if !top_level {
            return true;
        }
        current = parent;
    }
    false
}

fn extract_with_treesitter(
    source: &[u8],
    path: &Path,
) -> Result<Option<(ClassInfo, bool)>, String> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_php::language_php())
        .map_err(|e| format!("set_language: {e}"))?;

    let tree = parser
        .parse(source, None)
        .ok_or_else(|| "tree-sitter returned None".to_string())?;

    let root = tree.root_node();
    if root.has_error() {
        return Err("tree-sitter-php found a syntax error".into());
    }
    let src = source;

    let namespace = find_namespace(&root, src);

    // Find class/interface/trait/enum declaration
    let class_node = find_class_node(&root);
    let class_node = match class_node {
        Some(n) => n,
        None => return Ok(None),
    };
    let use_map = collect_use_map(root, class_node, src);

    let node_kind = class_node.kind();

    // enum → NoClass
    if node_kind == "enum_declaration" {
        return Ok(None);
    }

    let mut kind = match node_kind {
        "class_declaration" => ClassKind::Class,
        "interface_declaration" => ClassKind::Interface,
        "trait_declaration" => ClassKind::Trait,
        _ => ClassKind::Class,
    };

    let mut is_abstract = false;
    let mut is_final = false;

    // Check modifiers
    if let Some(modifiers_node) = class_node.child_by_field_name("modifier") {
        let mod_text = node_text(modifiers_node, src);
        if mod_text.contains("abstract") {
            is_abstract = true;
        }
        if mod_text.contains("final") {
            is_final = true;
        }
    }
    // Also check individual modifier nodes in children
    for child in iter_children(class_node) {
        if child.kind() == "abstract_modifier" {
            is_abstract = true;
        }
        if child.kind() == "final_modifier" {
            is_final = true;
        }
    }

    // As Tier 1: an abstract class has its own kind.
    if is_abstract && kind == ClassKind::Class {
        kind = ClassKind::AbstractClass;
    }

    let name = class_node
        .child_by_field_name("name")
        .map(|n| node_text(n, src).trim().to_string())
        .unwrap_or_default();

    if name.is_empty() {
        return Ok(None); // anonymous class
    }

    // tree-sitter-php gives `extends` and `implements` as plain children, not as fields. Tier 1
    // reads an interface's first parent as `extends` and the others as `implements`.
    let mut parents: Vec<String> = Vec::new();
    let mut implements: Vec<String> = Vec::new();
    for child in iter_children(class_node) {
        match child.kind() {
            "base_clause" => collect_names(child, src, &namespace, &use_map, &mut parents),
            "class_interface_clause" => {
                collect_names(child, src, &namespace, &use_map, &mut implements)
            }
            _ => {}
        }
    }
    let mut parents = parents.into_iter();
    let extends = parents.next();
    implements.extend(parents);

    // Find class body
    let body_node = class_node.child_by_field_name("body");

    let mut constructor: Option<Constructor> = None;
    let mut public_methods: Vec<MethodSignature> = Vec::new();
    let mut traits = TraitData::default();
    let is_trait = matches!(kind, ClassKind::Trait);

    if let Some(body) = body_node {
        for member in iter_children(body) {
            if member.kind() == "use_declaration" {
                crate::lexer::Lexer::read_trait_use_text(
                    node_text(member, src),
                    &namespace,
                    &use_map,
                    &mut traits,
                )
                .map_err(|e| format!("trait use: {e}"))?;
            }
            if member.kind() == "method_declaration" {
                let method_name = member
                    .child_by_field_name("name")
                    .map(|n| node_text(n, src).trim().to_string())
                    .unwrap_or_default();

                let is_abstract_method = has_modifier(member, src, "abstract");
                if method_name == "__construct" {
                    // An abstract constructor in a trait only asks for one. It is not one.
                    if !(is_trait && is_abstract_method) {
                        constructor = Some(parse_constructor(member, src, &namespace, &use_map));
                    }
                } else {
                    // Check visibility
                    let is_public = get_visibility(member, src) == "public";
                    let is_final_method = has_modifier(member, src, "final");
                    if is_trait || (is_public && !is_final_method) {
                        let is_static = has_modifier(member, src, "static");
                        let returns_reference = node_text(member, src).contains("function &");
                        let params = parse_method_params_ts(member, src, &namespace, &use_map);
                        let return_type = member.child_by_field_name("return_type").map(|n| {
                            resolve_type(
                                node_text(n, src).trim_start_matches(':'),
                                &namespace,
                                &use_map,
                            )
                        });
                        let signature = MethodSignature {
                            name: method_name,
                            params,
                            return_type,
                            is_static,
                            returns_reference,
                        };
                        if is_trait {
                            traits.methods.push(TraitMethod {
                                signature: signature.clone(),
                                is_public,
                                is_final: is_final_method,
                                is_abstract: is_abstract_method,
                            });
                        }
                        if is_public && !is_final_method {
                            public_methods.push(signature);
                        } else {
                            traits.hidden_methods.push(signature.name);
                        }
                    } else {
                        traits.hidden_methods.push(method_name);
                    }
                }
            }
        }
    }

    let fqcn = if namespace.is_empty() {
        name.clone()
    } else {
        format!("{}\\{}", namespace, name)
    };

    let inside_block = is_inside_block(class_node);
    Ok(Some((
        ClassInfo {
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
            traits: Box::new(traits),
        },
        inside_block,
    )))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn node_text<'a>(node: Node, src: &'a [u8]) -> &'a str {
    node.utf8_text(src).unwrap_or("")
}

/// The class imports in effect at the declaration: lowercase alias → name. Read from the tree, so
/// keyword case (`AS`), comments and line breaks do not matter. A namespace starts its own list.
fn collect_use_map(
    root: Node,
    class_node: Node,
    src: &[u8],
) -> rustc_hash::FxHashMap<String, String> {
    let mut map = rustc_hash::FxHashMap::default();
    let before_class = |node: &Node| node.start_byte() < class_node.start_byte();
    for child in iter_children(root).filter(before_class) {
        match child.kind() {
            "namespace_definition" => {
                map.clear();
                // A braced namespace holds its imports and the class in its body.
                if let Some(body) = child.child_by_field_name("body") {
                    for statement in iter_children(body).filter(before_class) {
                        if statement.kind() == "namespace_use_declaration" {
                            add_imports(statement, src, &mut map);
                        }
                    }
                }
            }
            "namespace_use_declaration" => add_imports(child, src, &mut map),
            _ => {}
        }
    }
    map
}

/// One `use` statement: single and comma-separated names, and groups. `use function` and
/// `use const` import no classes.
fn add_imports(declaration: Node, src: &[u8], map: &mut rustc_hash::FxHashMap<String, String>) {
    let imports_no_class =
        |node: Node| iter_children(node).any(|c| matches!(c.kind(), "function" | "const"));
    if imports_no_class(declaration) {
        return;
    }
    let mut add = |name: &str, alias: Option<&str>| {
        let name = normalize_fqn(name.trim());
        let alias = alias
            .map(str::to_string)
            .unwrap_or_else(|| name.rsplit('\\').next().unwrap_or(&name).to_string());
        if !name.is_empty() && !alias.is_empty() {
            map.insert(alias.to_ascii_lowercase(), name);
        }
    };
    let alias_of = |clause: Node| {
        iter_children(clause)
            .find(|c| c.kind() == "namespace_aliasing_clause")
            .and_then(|aliasing| iter_children(aliasing).find(|c| c.kind() == "name"))
            .map(|name| node_text(name, src).trim().to_string())
    };
    let mut prefix = String::new();
    for child in iter_children(declaration) {
        match child.kind() {
            "namespace_use_clause" => {
                let name = iter_children(child)
                    .find(|c| matches!(c.kind(), "qualified_name" | "name" | "namespace_name"))
                    .map(|n| node_text(n, src).to_string())
                    .unwrap_or_default();
                add(&name, alias_of(child).as_deref());
            }
            // `use A\B\{C, D as E}`: the prefix comes before the group.
            "namespace_name" | "qualified_name" => prefix = node_text(child, src).to_string(),
            "namespace_use_group" => {
                for clause in iter_children(child) {
                    if clause.kind() != "namespace_use_group_clause" || imports_no_class(clause) {
                        continue;
                    }
                    let Some(name) = iter_children(clause)
                        .find(|c| matches!(c.kind(), "namespace_name" | "qualified_name" | "name"))
                    else {
                        continue;
                    };
                    let full = format!(
                        "{}\\{}",
                        prefix.trim().trim_end_matches('\\'),
                        node_text(name, src).trim()
                    );
                    add(&full, alias_of(clause).as_deref());
                }
            }
            _ => {}
        }
    }
}

/// Resolve a type hint using the namespace and use-import map.
fn resolve_type_hint(
    raw: &str,
    namespace: &str,
    use_map: &rustc_hash::FxHashMap<String, String>,
) -> String {
    if raw.is_empty() {
        return raw.to_string();
    }
    // Already fully-qualified (starts with \)
    if raw.starts_with('\\') {
        return normalize_fqn(raw);
    }
    // `namespace\Foo` is relative to the current namespace.
    if let Some(rest) = strip_namespace_keyword(raw) {
        return if namespace.is_empty() {
            rest.to_string()
        } else {
            format!("{namespace}\\{rest}")
        };
    }
    let first = raw.split('\\').next().unwrap_or(raw);
    if let Some(mapped) = use_map.get(&first.to_ascii_lowercase()) {
        if raw.contains('\\') {
            let rest = &raw[first.len() + 1..];
            return format!("{}\\{}", mapped, rest);
        }
        return mapped.clone();
    }
    // Relative to current namespace
    if namespace.is_empty() {
        raw.to_string()
    } else {
        format!("{}\\{}", namespace, raw)
    }
}

fn find_namespace(root: &Node, src: &[u8]) -> String {
    for child in iter_children(*root) {
        if child.kind() == "namespace_definition" {
            if let Some(name_node) = child.child_by_field_name("name") {
                return normalize_fqn(node_text(name_node, src).trim());
            }
        }
    }
    String::new()
}

fn find_class_node<'a>(root: &'a Node<'a>) -> Option<Node<'a>> {
    // Search recursively (not too deep — top-level only)
    for child in iter_children(*root) {
        match child.kind() {
            "class_declaration"
            | "interface_declaration"
            | "trait_declaration"
            | "enum_declaration" => return Some(child),
            _ => {}
        }
    }
    // One level deeper (inside namespace body)
    for child in iter_children(*root) {
        if child.kind() == "namespace_definition" || child.kind() == "compound_statement" {
            for inner in iter_children(child) {
                match inner.kind() {
                    "class_declaration"
                    | "interface_declaration"
                    | "trait_declaration"
                    | "enum_declaration" => return Some(inner),
                    _ => {}
                }
            }
        }
    }
    None
}

/// The class names of a header clause, resolved as Tier 1 resolves them.
fn collect_names(
    node: Node,
    src: &[u8],
    namespace: &str,
    use_map: &rustc_hash::FxHashMap<String, String>,
    out: &mut Vec<String>,
) {
    for child in iter_children(node) {
        match child.kind() {
            "qualified_name" | "name" | "relative_name" => {
                let name = resolve_type(node_text(child, src), namespace, use_map);
                if !name.is_empty() {
                    out.push(name);
                }
            }
            _ => {}
        }
    }
}

fn get_visibility(member: Node, src: &[u8]) -> &'static str {
    for child in iter_children(member) {
        match node_text(child, src).trim() {
            "public" => return "public",
            "private" => return "private",
            "protected" => return "protected",
            _ => {}
        }
    }
    "public" // default in PHP
}

fn has_modifier(member: Node, src: &[u8], modifier: &str) -> bool {
    for child in iter_children(member) {
        if node_text(child, src).trim() == modifier {
            return true;
        }
    }
    false
}

fn parse_constructor(
    method: Node,
    src: &[u8],
    namespace: &str,
    use_map: &rustc_hash::FxHashMap<String, String>,
) -> Constructor {
    let params = if let Some(params_node) = method.child_by_field_name("parameters") {
        parse_ctor_params_ts(params_node, src, namespace, use_map)
    } else {
        Vec::new()
    };
    Constructor { params }
}

fn parse_ctor_params_ts(
    params_node: Node,
    src: &[u8],
    namespace: &str,
    use_map: &rustc_hash::FxHashMap<String, String>,
) -> Vec<ConstructorParam> {
    let mut params = Vec::new();
    for child in iter_children(params_node) {
        match child.kind() {
            "simple_parameter" | "variadic_parameter" | "property_promotion_parameter" => {
                let is_promoted = child.kind() == "property_promotion_parameter";
                let is_variadic = child.kind() == "variadic_parameter";

                let type_hint = child
                    .child_by_field_name("type")
                    .map(|n| resolve_type(node_text(n, src), namespace, use_map));

                let name = child
                    .child_by_field_name("name")
                    .map(|n| {
                        let s = node_text(n, src).trim();
                        // In a promoted parameter the name node includes a by-reference `&`.
                        s.trim_start_matches(['&', ' ', '$']).to_string()
                    })
                    .unwrap_or_default();

                let has_default = child.child_by_field_name("default_value").is_some();
                let default_value = child
                    .child_by_field_name("default_value")
                    .map(|n| node_text(n, src).trim().to_string())
                    .filter(|v| !v.is_empty());
                let is_primitive = type_hint.as_deref().map(is_primitive_type).unwrap_or(true);

                params.push(ConstructorParam {
                    name,
                    type_hint,
                    is_optional: has_default,
                    default_value,
                    is_primitive,
                    is_variadic,
                    is_promoted,
                });
            }
            _ => {}
        }
    }
    params
}

fn parse_method_params_ts(
    method: Node,
    src: &[u8],
    namespace: &str,
    use_map: &rustc_hash::FxHashMap<String, String>,
) -> Vec<MethodParam> {
    let mut params = Vec::new();
    if let Some(params_node) = method.child_by_field_name("parameters") {
        for child in iter_children(params_node) {
            match child.kind() {
                "simple_parameter" | "variadic_parameter" | "property_promotion_parameter" => {
                    let is_variadic = child.kind() == "variadic_parameter";
                    let type_hint = child
                        .child_by_field_name("type")
                        .map(|n| resolve_type(node_text(n, src), namespace, use_map));
                    let name = child
                        .child_by_field_name("name")
                        .map(|n| node_text(n, src).trim().trim_start_matches('$').to_string())
                        .unwrap_or_default();
                    let has_default = child.child_by_field_name("default_value").is_some();
                    let default_value = child
                        .child_by_field_name("default_value")
                        .map(|n| node_text(n, src).trim().to_string())
                        .filter(|v| !v.is_empty());
                    // `&` before the name. The type's own `&`s are part of an intersection.
                    let is_by_ref = child.child_by_field_name("reference_modifier").is_some();
                    params.push(MethodParam {
                        name,
                        type_hint,
                        has_default,
                        default_value,
                        is_variadic,
                        is_by_ref,
                    });
                }
                _ => {}
            }
        }
    }
    params
}

/// A type as Tier 1 writes it: each name resolved against the namespace and the `use` imports,
/// builtin names in lowercase, `?`, `|`, `&` and the parentheses of a DNF group kept, and
/// whitespace and comments dropped.
fn resolve_type(
    raw: &str,
    namespace: &str,
    use_map: &rustc_hash::FxHashMap<String, String>,
) -> String {
    let text = without_comments(raw);
    let mut out = String::new();
    let mut name = String::new();
    let flush = |name: &mut String, out: &mut String| {
        if name.is_empty() {
            return;
        }
        if let Some(absolute) = name.strip_prefix('\\') {
            out.push_str(absolute);
        } else if is_primitive_type(name) {
            out.push_str(&name.to_ascii_lowercase());
        } else {
            out.push_str(&resolve_type_hint(name, namespace, use_map));
        }
        name.clear();
    };
    for c in text.chars() {
        if matches!(c, '?' | '|' | '&' | '(' | ')') {
            flush(&mut name, &mut out);
            out.push(c);
        } else if c.is_whitespace() {
            flush(&mut name, &mut out);
        } else {
            name.push(c);
        }
    }
    flush(&mut name, &mut out);
    out
}

/// `/* … */`, `// …` and `# …` comments, which a type's text can hold around `|`.
fn without_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("/*") {
            rest = after.find("*/").map_or("", |end| &after[end + 2..]);
            out.push(' ');
        } else if rest.starts_with("//") || (rest.starts_with('#') && !rest.starts_with("#[")) {
            rest = rest.find('\n').map_or("", |end| &rest[end..]);
        } else {
            let c = rest.chars().next().unwrap();
            out.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    out
}

/// The rest of `namespace\Foo`, in any case of the keyword.
pub(crate) fn strip_namespace_keyword(name: &str) -> Option<&str> {
    let prefix = name.get(..10)?;
    prefix
        .eq_ignore_ascii_case("namespace\\")
        .then(|| &name[10..])
}

fn normalize_fqn(s: &str) -> String {
    s.trim_start_matches('\\').to_string()
}

fn is_primitive_type(t: &str) -> bool {
    let t = t.trim_start_matches('?');
    if t.contains('|') {
        return t.split('|').all(is_primitive_base);
    }
    is_primitive_base(t)
}

/// PHP reads a reserved type name in any case. `integer`, `double` and `boolean` are not reserved,
/// so `Boolean` can be a class. Those three keep their exact spelling here.
fn is_primitive_base(t: &str) -> bool {
    if matches!(t, "integer" | "double" | "boolean") {
        return true;
    }
    matches!(
        t.to_ascii_lowercase().as_str(),
        "int"
            | "float"
            | "string"
            | "bool"
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

fn iter_children(node: Node) -> impl Iterator<Item = Node> {
    let count = node.child_count();
    (0..count).filter_map(move |i| node.child(i))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn extract(php: &str) -> ExtractResult {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(php.as_bytes()).unwrap();
        extract_tier2(f.path())
    }

    #[test]
    fn test_tier2_reserved_type_names_are_builtin_in_any_case() {
        let result = extract(
            "<?php\nnamespace Foo;\nclass Baz\n{\n    public function __construct(Array $a, ?String $b, Int|Null $c, Boolean $d) {}\n}\n",
        );
        let ExtractResult::Ok(info) = result else {
            panic!("Expected Ok, got {result:?}");
        };
        let hints: Vec<_> = info
            .constructor
            .unwrap()
            .params
            .iter()
            .map(|param| (param.type_hint.clone().unwrap(), param.is_primitive))
            .collect();
        assert_eq!(
            hints,
            [
                ("array".to_string(), true),
                ("?string".to_string(), true),
                ("int|null".to_string(), true),
                ("Foo\\Boolean".to_string(), false),
            ]
        );
    }

    #[test]
    fn test_tier2_simple_class() {
        let result = extract("<?php\nnamespace Foo\\Bar;\nclass Baz {}");
        match result {
            ExtractResult::Ok(info) => {
                assert_eq!(info.fqcn, "Foo\\Bar\\Baz");
            }
            other => panic!("Expected Ok, got {:?}", other),
        }
    }

    fn extract_with_facts(php: &str) -> (ExtractResult, ScanFacts) {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(php.as_bytes()).unwrap();
        extract_tier2_with_facts(f.path())
    }

    const HOOKED: &str = "class Hooked {\n    public string $label { get => 'x'; }\n    public function __construct(private A&B $both, int $size = 3) {}\n}";

    #[test]
    fn a_tree_with_a_syntax_error_goes_on_to_the_next_tier() {
        // tree-sitter-php 0.22 does not know property hooks. Its partial tree used to give the
        // class without its constructor.
        match extract(&format!("<?php\nnamespace Acme;\n{HOOKED}\n")) {
            ExtractResult::ParseFailure(reason) => assert!(reason.contains("syntax error")),
            other => panic!("expected ParseFailure, got {other:?}"),
        }
    }

    #[test]
    fn a_tree_with_a_syntax_error_and_no_class_goes_on_too() {
        // Inside `if`, Tier 2 finds no declaration; before, that was `NoClass`.
        let result = extract(&format!(
            "<?php\nnamespace Acme;\nif (true) {{\n{HOOKED}\n}}\n"
        ));
        assert!(
            matches!(result, ExtractResult::ParseFailure(_)),
            "{result:?}"
        );
    }

    #[test]
    fn a_class_in_a_block_is_reported_inside_one() {
        let (result, facts) = extract_with_facts(
            "<?php\nnamespace Acme;\n{\n    class Blocked { public function __construct(A&B $x) {} }\n}\n",
        );
        assert!(matches!(result, ExtractResult::Ok(_)), "{result:?}");
        assert!(facts.inside_block);
        assert!(!facts.class_check);

        let (result, facts) = extract_with_facts(
            "<?php\nnamespace Acme;\nclass Top { public function __construct(A&B $x) {} }\n",
        );
        assert!(matches!(result, ExtractResult::Ok(_)), "{result:?}");
        assert!(!facts.inside_block);
    }

    #[test]
    fn test_tier2_intersection_type() {
        // Intersection type that Tier 1 can't handle
        let result = extract(
            "<?php\nnamespace Foo;\nclass Bar {\n    public function __construct(Baz&Qux $x) {}\n}",
        );
        match result {
            ExtractResult::Ok(info) => {
                let ctor = info.constructor.unwrap();
                // tree-sitter parses it; type may be extracted or empty
                assert_eq!(ctor.params[0].name, "x");
            }
            other => panic!(
                "Expected Ok from tier2 for intersection type, got {:?}",
                other
            ),
        }
    }

    #[test]
    fn test_tier2_reads_promoted_constructor_parameters() {
        // The intersection type sends this file from Tier 1 to Tier 2.
        let result = extract(
            "<?php\nnamespace Foo;\nclass Bar {\n    public function __construct(\n        private readonly Baz&Qux $x,\n        Dep $y,\n        protected ?Dep $z = null,\n        public int &$r = 0,\n    ) {}\n}",
        );
        let ExtractResult::Ok(info) = result else {
            panic!("Expected Ok, got {:?}", result);
        };
        let params = info.constructor.unwrap().params;
        let names: Vec<_> = params.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["x", "y", "z", "r"]);
        let promoted: Vec<_> = params.iter().map(|p| p.is_promoted).collect();
        assert_eq!(promoted, [true, false, true, true]);
        assert!(!params[0].is_optional);
        assert_eq!(params[2].default_value.as_deref(), Some("null"));
        assert!(params[2].is_optional);
        assert_eq!(params[2].type_hint.as_deref(), Some("?Foo\\Dep"));
        assert!(params[3].is_primitive);
    }

    #[test]
    fn test_tier2_nullable_parameter_without_default_is_required() {
        let result = extract(
            "<?php\nnamespace Foo;\nclass Bar {\n    public function __construct(\n        private readonly Baz&Qux $x,\n        ?Dep $y,\n    ) {}\n}",
        );
        let ExtractResult::Ok(info) = result else {
            panic!("Expected Ok, got {:?}", result);
        };
        let params = info.constructor.unwrap().params;
        assert_eq!(params[1].type_hint.as_deref(), Some("?Foo\\Dep"));
        assert!(!params[1].is_optional);
    }
}
