//! `Magento\Framework\Code\GeneratedFiles::cleanGeneratedFiles()`, which Magento's
//! `ObjectManagerFactory::create()` runs on every boot, so also before `setup:di:compile`.
//!
//! `magento/magento-composer-installer` touches `var/.regenerate` after each `composer install`
//! and `composer update`. When Magento boots and finds that flag, it deletes the flag,
//! `generated/code`, `generated/metadata` and `var/cache`. This compiler does not boot Magento,
//! so without this the flag outlives the compile, and the first PHP process that boots the store
//! afterwards deletes the output this compiler just wrote.
//!
//! Magento skips the clean while another process holds `var/.regenerate.lock`. No other Magento
//! process runs during a compile, so that lock is not checked here.

use std::fs;
use std::io::ErrorKind;
use std::path::Path;

const FLAG: &str = "var/.regenerate";
const FOLDERS: [&str; 3] = ["generated/code", "generated/metadata", "var/cache"];

/// Does what Magento's boot does when `composer install` asked for a regeneration. Returns
/// whether the flag was there.
pub fn clean(magento_root: &Path) -> bool {
    let flag = magento_root.join(FLAG);
    if !flag.is_file() {
        return false;
    }
    let mut result = fs::remove_file(&flag);
    for folder in FOLDERS {
        if result.is_err() {
            break;
        }
        result = match fs::remove_dir_all(magento_root.join(folder)) {
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            other => other,
        };
    }
    if let Err(error) = result {
        // Magento asks for the regeneration again when a folder cannot be deleted.
        log::warn!("could not clean the generated files that {FLAG} asks for: {error}");
        let _ = fs::write(&flag, "");
    }
    true
}
