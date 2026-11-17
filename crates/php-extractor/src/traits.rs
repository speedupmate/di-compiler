//! Applies PHP's trait rules, so that `ClassInfo` holds what `ReflectionClass` sees.
//!
//! The order is the one of `zend_add_trait_method()`: a method of the class itself wins,
//! then a method from a trait, then an inherited one. An abstract trait method never
//! replaces anything. It only asks for a method.

use rustc_hash::{FxHashMap, FxHashSet};

use crate::types::{ClassInfo, ClassKind, Constructor, MethodSignature, TraitMethod, TraitRule};

#[derive(Clone, Default)]
struct Members {
    methods: Vec<TraitMethod>,
    constructor: Option<Constructor>,
}

/// Adds the members of used traits to every class in `class_map` that has any.
/// `lookup` finds traits outside the map. A trait that nobody can find adds nothing.
/// Running this twice is safe: applied classes forget their `use` statements.
pub fn apply_traits(
    class_map: &mut FxHashMap<String, ClassInfo>,
    lookup: &dyn Fn(&str) -> Option<ClassInfo>,
) {
    let mut memo: FxHashMap<String, Members> = FxHashMap::default();
    let mut visiting: FxHashSet<String> = FxHashSet::default();
    let mut found: Vec<(String, Members)> = Vec::new();
    for (fqcn, info) in class_map.iter() {
        if info.traits.uses.is_empty() || matches!(info.kind, ClassKind::Trait) {
            continue;
        }
        let used = from_used_traits(info, class_map, lookup, &mut memo, &mut visiting);
        found.push((fqcn.clone(), used));
    }

    for (fqcn, used) in found {
        let Some(info) = class_map.get_mut(&fqcn) else {
            continue;
        };
        add_to_class(info, used);
    }
}

fn add_to_class(info: &mut ClassInfo, used: Members) {
    let mut declared: FxHashSet<String> = info
        .public_methods
        .iter()
        .map(|method| method.name.as_str())
        .chain(info.traits.hidden_methods.iter().map(String::as_str))
        .map(str::to_ascii_lowercase)
        .collect();
    for method in used.methods {
        if method.is_abstract || !declared.insert(method.signature.name.to_ascii_lowercase()) {
            continue;
        }
        if method.is_public && !method.is_final {
            info.public_methods.push(method.signature);
        } else {
            info.traits.hidden_methods.push(method.signature.name);
        }
    }
    if info.constructor.is_none() {
        info.constructor = used.constructor;
    }
    info.traits.uses.clear();
}

/// All members of a trait: its own, and those of the traits it uses.
fn members_of_trait(
    name: &str,
    class_map: &FxHashMap<String, ClassInfo>,
    lookup: &dyn Fn(&str) -> Option<ClassInfo>,
    memo: &mut FxHashMap<String, Members>,
    visiting: &mut FxHashSet<String>,
) -> Members {
    let name = name.trim_start_matches('\\');
    if let Some(members) = memo.get(name) {
        return members.clone();
    }
    if !visiting.insert(name.to_string()) {
        return Members::default();
    }
    let looked_up;
    let info = match class_map.get(name) {
        Some(info) => Some(info),
        None => {
            looked_up = lookup(name);
            looked_up.as_ref()
        }
    };
    let members = match info {
        Some(info) => {
            let used = from_used_traits(info, class_map, lookup, memo, visiting);
            let mut methods = info.traits.methods.clone();
            for method in used.methods {
                add_method(&mut methods, method, true);
            }
            Members {
                methods,
                constructor: info.constructor.clone().or(used.constructor),
            }
        }
        None => {
            log::debug!("trait {name} not found");
            Members::default()
        }
    };
    visiting.remove(name);
    memo.insert(name.to_string(), members.clone());
    members
}

/// `zend_traits_init_trait_structures()` and `zend_traits_copy_functions()`: for each used
/// trait, in order, the aliases of a method come first and then the method, unless an
/// `insteadof` excluded it. Rules of all `use` statements of a class apply to all its traits.
fn from_used_traits(
    info: &ClassInfo,
    class_map: &FxHashMap<String, ClassInfo>,
    lookup: &dyn Fn(&str) -> Option<ClassInfo>,
    memo: &mut FxHashMap<String, Members>,
    visiting: &mut FxHashSet<String>,
) -> Members {
    let mut result = Members::default();
    for trait_name in &info.traits.uses {
        let members = members_of_trait(trait_name, class_map, lookup, memo, visiting);
        for method in &members.methods {
            for rule in &info.traits.rules {
                if let Some(copy) = aliased_copy(rule, trait_name, method) {
                    add_method(&mut result.methods, copy, false);
                }
            }
            if !is_excluded(&info.traits.rules, trait_name, &method.signature.name) {
                let mut method = method.clone();
                for rule in &info.traits.rules {
                    change_modifiers(rule, trait_name, &mut method);
                }
                add_method(&mut result.methods, method, false);
            }
        }
        if result.constructor.is_none()
            && !is_excluded(&info.traits.rules, trait_name, "__construct")
        {
            result.constructor = members.constructor;
        }
    }
    result
}

/// `keep_first` is for a trait's own methods, which beat those it uses. Between two used
/// traits a real method replaces an abstract one.
fn add_method(methods: &mut Vec<TraitMethod>, method: TraitMethod, keep_first: bool) {
    let existing = methods.iter_mut().find(|known| {
        known
            .signature
            .name
            .eq_ignore_ascii_case(&method.signature.name)
    });
    match existing {
        None => methods.push(method),
        Some(known) => {
            if !keep_first && known.is_abstract && !method.is_abstract {
                *known = method;
            }
        }
    }
}

fn same_name(left: &str, right: &str) -> bool {
    left.trim_start_matches('\\')
        .eq_ignore_ascii_case(right.trim_start_matches('\\'))
}

fn is_excluded(rules: &[TraitRule], trait_name: &str, method: &str) -> bool {
    rules.iter().any(|rule| match rule {
        TraitRule::InsteadOf {
            method: rule_method,
            excluded,
        } => {
            rule_method.eq_ignore_ascii_case(method)
                && excluded.iter().any(|name| same_name(name, trait_name))
        }
        TraitRule::Alias { .. } => false,
    })
}

fn rule_matches(
    rule_trait: &Option<String>,
    rule_method: &str,
    trait_name: &str,
    method: &MethodSignature,
) -> bool {
    rule_method.eq_ignore_ascii_case(&method.name)
        && rule_trait
            .as_deref()
            .is_none_or(|name| same_name(name, trait_name))
}

fn aliased_copy(rule: &TraitRule, trait_name: &str, method: &TraitMethod) -> Option<TraitMethod> {
    let TraitRule::Alias {
        trait_name: rule_trait,
        method: rule_method,
        alias: Some(alias),
        is_public,
        is_final,
    } = rule
    else {
        return None;
    };
    if !rule_matches(rule_trait, rule_method, trait_name, &method.signature) {
        return None;
    }
    let mut copy = method.clone();
    copy.signature.name = alias.clone();
    copy.is_public = is_public.unwrap_or(copy.is_public);
    copy.is_final |= is_final;
    Some(copy)
}

fn change_modifiers(rule: &TraitRule, trait_name: &str, method: &mut TraitMethod) {
    let TraitRule::Alias {
        trait_name: rule_trait,
        method: rule_method,
        alias: None,
        is_public,
        is_final,
    } = rule
    else {
        return;
    };
    if rule_matches(rule_trait, rule_method, trait_name, &method.signature) {
        method.is_public = is_public.unwrap_or(method.is_public);
        method.is_final |= is_final;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;
    use std::io::Write;

    fn extract(php: &str) -> ClassInfo {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(php.as_bytes()).unwrap();
        Lexer::extract(file.path()).unwrap()
    }

    fn map_of(sources: &[&str]) -> FxHashMap<String, ClassInfo> {
        let mut map = FxHashMap::default();
        for source in sources {
            let info = extract(&format!("<?php\nnamespace App;\n{source}\n"));
            map.insert(info.fqcn.clone(), info);
        }
        map
    }

    fn public_names(map: &FxHashMap<String, ClassInfo>, fqcn: &str) -> Vec<String> {
        let mut names: Vec<String> = map[fqcn]
            .public_methods
            .iter()
            .map(|method| method.name.clone())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn method_of_the_class_wins_over_the_trait() {
        let mut map = map_of(&[
            "trait T { public function run(): int { return 1; } }",
            "class C { use T; public function run(): string { return ''; } }",
        ]);
        apply_traits(&mut map, &|_| None);
        let run = &map["App\\C"].public_methods[0];
        assert_eq!(run.return_type.as_deref(), Some("string"));
        assert_eq!(map["App\\C"].public_methods.len(), 1);
    }

    #[test]
    fn non_public_method_of_the_class_wins_over_a_public_trait_method() {
        let mut map = map_of(&[
            "trait T { public function run() {} }",
            "class C { use T; protected function run() {} }",
        ]);
        apply_traits(&mut map, &|_| None);
        assert!(public_names(&map, "App\\C").is_empty());
    }

    #[test]
    fn insteadof_decides_between_two_traits() {
        let mut map = map_of(&[
            "trait A { public function run(): int { return 1; } }",
            "trait B { public function run(): string { return ''; } }",
            "class C { use A, B { B::run insteadof A; } }",
        ]);
        apply_traits(&mut map, &|_| None);
        let methods = &map["App\\C"].public_methods;
        assert_eq!(methods.len(), 1);
        assert_eq!(methods[0].return_type.as_deref(), Some("string"));
    }

    #[test]
    fn alias_adds_a_method_and_can_change_visibility() {
        let mut map = map_of(&[
            "trait A { public function run() {} protected function hide() {} }",
            "class C { use A { run as walk; run as protected crawl; hide as public shown; } }",
        ]);
        apply_traits(&mut map, &|_| None);
        assert_eq!(public_names(&map, "App\\C"), ["run", "shown", "walk"]);
        assert!(map["App\\C"]
            .traits
            .hidden_methods
            .contains(&"crawl".to_string()));
    }

    #[test]
    fn as_without_a_new_name_changes_visibility_of_the_method_itself() {
        let mut map = map_of(&[
            "trait A { public function run() {} protected function open() {} }",
            "class C { use A { run as protected; open as public; } }",
        ]);
        apply_traits(&mut map, &|_| None);
        assert_eq!(public_names(&map, "App\\C"), ["open"]);
    }

    #[test]
    fn real_method_replaces_an_abstract_one_from_another_trait() {
        let mut map = map_of(&[
            "trait A { abstract public function run(): int; }",
            "trait B { public function run(): int { return 1; } }",
            "class C { use A, B; }",
        ]);
        apply_traits(&mut map, &|_| None);
        assert_eq!(public_names(&map, "App\\C"), ["run"]);
    }

    #[test]
    fn trait_outside_the_map_comes_from_lookup() {
        let outside = extract("<?php\nnamespace Lib;\ntrait T { public function run() {} }\n");
        let mut map = map_of(&["class C { use \\Lib\\T; }"]);
        apply_traits(&mut map, &|name| {
            (name == "Lib\\T").then(|| outside.clone())
        });
        assert_eq!(public_names(&map, "App\\C"), ["run"]);
    }

    #[test]
    fn trait_nobody_can_find_adds_nothing() {
        let mut map = map_of(&["class C { use Gone; public function run() {} }"]);
        apply_traits(&mut map, &|_| None);
        assert_eq!(public_names(&map, "App\\C"), ["run"]);
    }

    #[test]
    fn traits_that_use_each_other_do_not_hang() {
        let mut map = map_of(&[
            "trait A { use B; public function a() {} }",
            "trait B { use A; public function b() {} }",
            "class C { use A; }",
        ]);
        apply_traits(&mut map, &|_| None);
        assert_eq!(public_names(&map, "App\\C"), ["a", "b"]);
    }

    #[test]
    fn applying_twice_changes_nothing() {
        let mut map = map_of(&["trait T { public function run() {} }", "class C { use T; }"]);
        apply_traits(&mut map, &|_| None);
        let once = map.clone();
        apply_traits(&mut map, &|_| None);
        assert_eq!(map, once);
    }

    #[test]
    fn tree_sitter_tier_reads_trait_use_too() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(
            b"<?php\nnamespace App;\nuse Lib\\T;\nclass C { use T, U { T::run as walk; } }\n",
        )
        .unwrap();
        let crate::types::ExtractResult::Ok(info) = crate::tier2::extract_tier2(file.path()) else {
            panic!("tier 2 failed");
        };
        assert_eq!(info.traits.uses, ["Lib\\T", "App\\U"]);
        assert_eq!(info.traits.rules.len(), 1);
    }

    #[test]
    fn tree_sitter_tier_records_all_methods_of_a_trait() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(
            b"<?php\nnamespace App;\ntrait T { protected function a() {} abstract public function b(); final public function c() {} }\n",
        )
        .unwrap();
        let crate::types::ExtractResult::Ok(info) = crate::tier2::extract_tier2(file.path()) else {
            panic!("tier 2 failed");
        };
        let flags: Vec<(bool, bool, bool)> = info
            .traits
            .methods
            .iter()
            .map(|m| (m.is_public, m.is_final, m.is_abstract))
            .collect();
        assert_eq!(
            flags,
            [
                (false, false, false),
                (true, false, true),
                (true, true, false)
            ]
        );
    }
}
