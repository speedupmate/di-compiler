//! Tier 2: tree-sitter-php fallback extractor.
//!
//! Used when the Tier 1 custom lexer encounters syntax it cannot handle (e.g. intersection types).
use std::path::Path;

use tree_sitter::{Node, Parser};

use crate::types::{
    ClassInfo, ClassKind, Constructor, ConstructorParam, ExtractResult, MethodParam,
    MethodSignature,
};

pub fn extract_tier2(path: &Path) -> ExtractResult {
    let source = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => return ExtractResult::PhpFallbackFailed(format!("io: {e}")),
    };

    match extract_with_treesitter(&source, path) {
        Ok(Some(info)) => ExtractResult::Ok(info),
        Ok(None) => ExtractResult::NoClass,
        Err(e) => ExtractResult::ParseFailure(e),
    }
}

fn extract_with_treesitter(source: &[u8], path: &Path) -> Result<Option<ClassInfo>, String> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_php::language_php())
        .map_err(|e| format!("set_language: {e}"))?;

    let tree = parser
        .parse(source, None)
        .ok_or_else(|| "tree-sitter returned None".to_string())?;

    let root = tree.root_node();
    let src = source;

    // Find namespace
    let namespace = find_namespace(&root, src);

    // Find class/interface/trait/enum declaration
    let class_node = find_class_node(&root);
    let class_node = match class_node {
        Some(n) => n,
        None => return Ok(None),
    };

    let node_kind = class_node.kind();

    // enum → NoClass
    if node_kind == "enum_declaration" {
        return Ok(None);
    }

    let kind = match node_kind {
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

    let name = class_node
        .child_by_field_name("name")
        .map(|n| node_text(n, src).trim().to_string())
        .unwrap_or_default();

    if name.is_empty() {
        return Ok(None); // anonymous class
    }

    let extends = class_node
        .child_by_field_name("base_clause")
        .or_else(|| class_node.child_by_field_name("extends_clause"))
        .map(|n| {
            // base_clause may have a name child
            n.child_by_field_name("name")
                .or_else(|| find_named_node(n, "qualified_name"))
                .or_else(|| find_named_node(n, "name"))
                .map(|nn| normalize_fqn(node_text(nn, src).trim()))
                .unwrap_or_else(|| {
                    normalize_fqn(
                        node_text(n, src)
                            .trim()
                            .trim_start_matches("extends")
                            .trim(),
                    )
                })
        });

    let mut implements: Vec<String> = Vec::new();
    if let Some(iface_clause) = class_node
        .child_by_field_name("class_implements")
        .or_else(|| class_node.child_by_field_name("implements_clause"))
    {
        collect_fqns(iface_clause, src, &mut implements);
    }

    // Find class body
    let body_node = class_node.child_by_field_name("body");

    let mut constructor: Option<Constructor> = None;
    let mut public_methods: Vec<MethodSignature> = Vec::new();

    if let Some(body) = body_node {
        for member in iter_children(body) {
            if member.kind() == "method_declaration" {
                let method_name = member
                    .child_by_field_name("name")
                    .map(|n| node_text(n, src).trim().to_string())
                    .unwrap_or_default();

                if method_name == "__construct" {
                    constructor = Some(parse_constructor(member, src));
                } else {
                    // Check visibility
                    let vis = get_visibility(member, src);
                    let is_final_method = has_modifier(member, src, "final");
                    if vis == "public" && !is_final_method {
                        let is_static = has_modifier(member, src, "static");
                        let returns_reference = node_text(member, src).contains("function &");
                        let params = parse_method_params_ts(member, src);
                        let return_type = member
                            .child_by_field_name("return_type")
                            .map(|n| node_text(n, src).trim_start_matches(':').trim().to_string());
                        public_methods.push(MethodSignature {
                            name: method_name,
                            params,
                            return_type,
                            is_static,
                            returns_reference,
                        });
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

    Ok(Some(ClassInfo {
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
    }))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn node_text<'a>(node: Node, src: &'a [u8]) -> &'a str {
    node.utf8_text(src).unwrap_or("")
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

fn find_named_node<'a>(parent: Node<'a>, kind: &str) -> Option<Node<'a>> {
    iter_children(parent).find(|&child| child.kind() == kind)
}

fn collect_fqns(node: Node, src: &[u8], out: &mut Vec<String>) {
    for child in iter_children(node) {
        match child.kind() {
            "qualified_name" | "name" => {
                let s = normalize_fqn(node_text(child, src).trim());
                if !s.is_empty() {
                    out.push(s);
                }
            }
            _ => collect_fqns(child, src, out),
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

fn parse_constructor(method: Node, src: &[u8]) -> Constructor {
    let params = if let Some(params_node) = method.child_by_field_name("parameters") {
        parse_ctor_params_ts(params_node, src)
    } else {
        Vec::new()
    };
    Constructor { params }
}

fn parse_ctor_params_ts(params_node: Node, src: &[u8]) -> Vec<ConstructorParam> {
    let mut params = Vec::new();
    for child in iter_children(params_node) {
        match child.kind() {
            "simple_parameter" | "variadic_parameter" | "promoted_parameter" => {
                let is_promoted = child.kind() == "promoted_parameter";
                let is_variadic = child.kind() == "variadic_parameter";

                let type_hint = child
                    .child_by_field_name("type")
                    .map(|n| extract_type_from_node(n, src));

                let name = child
                    .child_by_field_name("name")
                    .map(|n| {
                        let s = node_text(n, src).trim();
                        s.trim_start_matches('$').to_string()
                    })
                    .unwrap_or_default();

                let has_default = child.child_by_field_name("default_value").is_some();
                let default_value = child
                    .child_by_field_name("default_value")
                    .map(|n| node_text(n, src).trim().to_string())
                    .filter(|v| !v.is_empty());
                let is_nullable = child.child_by_field_name("nullable_type").is_some()
                    || type_hint
                        .as_ref()
                        .map(|t| t.starts_with('?'))
                        .unwrap_or(false);
                let is_primitive = type_hint.as_deref().map(is_primitive_type).unwrap_or(true);

                params.push(ConstructorParam {
                    name,
                    type_hint,
                    is_optional: has_default || is_nullable,
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

fn parse_method_params_ts(method: Node, src: &[u8]) -> Vec<MethodParam> {
    let mut params = Vec::new();
    if let Some(params_node) = method.child_by_field_name("parameters") {
        for child in iter_children(params_node) {
            match child.kind() {
                "simple_parameter" | "variadic_parameter" | "promoted_parameter" => {
                    let is_variadic = child.kind() == "variadic_parameter";
                    let type_hint = child
                        .child_by_field_name("type")
                        .map(|n| extract_type_from_node(n, src));
                    let name = child
                        .child_by_field_name("name")
                        .map(|n| node_text(n, src).trim().trim_start_matches('$').to_string())
                        .unwrap_or_default();
                    let has_default = child.child_by_field_name("default_value").is_some();
                    let default_value = child
                        .child_by_field_name("default_value")
                        .map(|n| node_text(n, src).trim().to_string())
                        .filter(|v| !v.is_empty());
                    let raw = node_text(child, src);
                    let is_by_ref = raw.contains("&$") || raw.trim_start().starts_with('&');
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

fn extract_type_from_node(type_node: Node, src: &[u8]) -> String {
    match type_node.kind() {
        "named_type" | "qualified_name" | "name" => normalize_fqn(node_text(type_node, src).trim()),
        "nullable_type" => {
            // `?TypeName`
            let inner = type_node
                .named_child(0)
                .map(|n| normalize_fqn(node_text(n, src).trim()))
                .unwrap_or_default();
            format!("?{inner}")
        }
        "union_type" => {
            // `Foo|Bar` — preserve all union parts in declaration order
            let mut parts = Vec::new();
            for child in iter_children(type_node) {
                match child.kind() {
                    "named_type" | "name" | "qualified_name" => {
                        parts.push(normalize_fqn(node_text(child, src).trim()));
                    }
                    "primitive_type" => {
                        parts.push(node_text(child, src).trim().to_string());
                    }
                    _ => {}
                }
            }
            parts.join("|")
        }
        _ => normalize_fqn(node_text(type_node, src).trim()),
    }
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
    fn test_tier2_simple_class() {
        let result = extract("<?php\nnamespace Foo\\Bar;\nclass Baz {}");
        match result {
            ExtractResult::Ok(info) => {
                assert_eq!(info.fqcn, "Foo\\Bar\\Baz");
            }
            other => panic!("Expected Ok, got {:?}", other),
        }
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
}
