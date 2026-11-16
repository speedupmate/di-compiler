//! Modules this compiler does not support. A store that has one installed, enabled or disabled,
//! has to use Magento's compile.
//!
//! A disabled module counts too: Magento 2.4.8 reads the configuration of a module that is
//! registered but disabled, so its preferences still change Magento's output. A module that
//! `app/etc/config.php` enables counts even when its package is missing. A module that is only
//! listed as disabled, with its package removed, does not count, because Magento has nothing of it
//! to read.
//!
//! This is a deliberate difference from Magento, whose compile runs fine with these modules. There
//! is no option to bypass it, because the output would not be what the store asked for.

use rustc_hash::{FxHashMap, FxHashSet};

struct IncompatibleModule {
    name: &'static str,
    package: &'static str,
    reason: &'static str,
}

const INCOMPATIBLE_MODULES: &[IncompatibleModule] = &[IncompatibleModule {
    name: "Creatuity_Interception",
    package: "creatuity/magento2-interceptors",
    reason:
        "it replaces Magento's Interceptor generator and this compiler does not generate its format",
}];

/// The message for the listed modules that are installed (`installed_modules`, by the name in
/// their `etc/module.xml`) or that `app/etc/config.php` enables, if there are any.
pub(crate) fn incompatible_modules(
    installed_modules: &FxHashSet<String>,
    enabled_modules: &FxHashMap<String, usize>,
) -> Option<String> {
    let found: Vec<String> = INCOMPATIBLE_MODULES
        .iter()
        .filter_map(|module| {
            let state = match (
                installed_modules.contains(module.name),
                enabled_modules.contains_key(module.name),
            ) {
                (true, true) => "is installed and enabled",
                (true, false) => "is installed but disabled",
                (false, true) => "is enabled in app/etc/config.php",
                (false, false) => return None,
            };
            Some(format!(
                "Module {} (package {}) {state} and is not supported: {}.",
                module.name, module.package, module.reason
            ))
        })
        .collect();
    (!found.is_empty()).then(|| {
        format!(
            "{}\nRemove the package with composer, or use bin/magento setup:di:compile for this store.",
            found.join("\n")
        )
    })
}
