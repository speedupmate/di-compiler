//! PHP snippets shared by every script that loads store classes: the class checks, the
//! reflection worker and Tier 3.

/// Sets up PHP as `bin/magento` does. Needs `$root` and `$generated`. Loaded class files can print.
pub const BOOT_PHP: &str = r#"
ob_start();
require is_file($root . '/app/bootstrap.php') ? $root . '/app/bootstrap.php' : $root . '/vendor/autoload.php';
while (ob_get_level() > 0) {
    ob_end_clean();
}
ini_set('display_errors', '0');
ini_set('memory_limit', '-1');
// `Code\Generator\Autoloader` creates these classes on demand when the source class exists.
spl_autoload_register(static function (string $class) use ($generated): void {
    $file = $generated . '/' . str_replace('\\', '/', $class) . '.php';
    if (is_file($file)) {
        require $file;
        return;
    }
    foreach (['\\Proxy', '\\Interceptor', '\\ProxyDeferred', '\\Remote', '\\Logger', 'Factory', 'Mapper',
              'Persistor', 'Repository', 'Converter', 'SearchResults', 'ExtensionInterface', 'Extension'] as $suffix) {
        if (!str_ends_with($class, $suffix)) {
            continue;
        }
        $source = substr($class, 0, -strlen($suffix));
        foreach ([$source, $source . 'Interface'] as $name) {
            if ($name === '' || $name === 'Interface' || (!class_exists($name) && !interface_exists($name))) {
                continue;
            }
            $split = strrpos($class, '\\');
            $namespace = $split === false ? '' : 'namespace ' . substr($class, 0, $split) . '; ';
            $short = substr($class, $split === false ? 0 : $split + 1);
            try {
                eval($namespace . ($suffix === 'ExtensionInterface' ? 'interface ' : 'class ') . $short . ' {}');
            } catch (\Throwable) {
            }
            return;
        }
    }
});
// `bin/magento` sets this handler. It makes an exception of every warning and deprecation that
// `app/bootstrap.php` lets through, so PHP compiling an old class file can stop the compile.
$errorHandler = 'Magento\\Framework\\App\\ErrorHandler';
if (class_exists($errorHandler)) {
    set_error_handler([new $errorHandler(), 'handler']);
}
"#;

/// `bin/magento` creates the real ObjectManager before it loads any class, so a class file can use
/// `ObjectManager::getInstance()` while it loads. Starting one takes seconds and few stores need
/// it, so this one starts the store's real one on first use. Needs `$root`. What it would write
/// goes to a temporary folder. A failure to start, or in the call itself, stays an error of the
/// class that asked, as it is in Magento.
pub const OBJECT_MANAGER_PHP: &str = r#"
if (class_exists('Magento\\Framework\\App\\ObjectManager') && class_exists('Magento\\Framework\\App\\Bootstrap')) {
    \Magento\Framework\App\ObjectManager::setInstance(new class($root) implements \Magento\Framework\ObjectManagerInterface {
        public function __construct(private string $root)
        {
        }

        private function real(): \Magento\Framework\ObjectManagerInterface
        {
            $level = ob_get_level();
            ob_start();
            try {
                if (is_file($this->root . '/app/bootstrap.php')) {
                    require_once $this->root . '/app/bootstrap.php';
                }
                $folder = sys_get_temp_dir() . '/fast-di-compile-' . uniqid((string) getmypid(), true);
                register_shutdown_function(static function () use ($folder): void {
                    if (!is_dir($folder)) {
                        return;
                    }
                    $items = new RecursiveIteratorIterator(
                        new RecursiveDirectoryIterator($folder, FilesystemIterator::SKIP_DOTS),
                        RecursiveIteratorIterator::CHILD_FIRST
                    );
                    foreach ($items as $item) {
                        $item->isDir() && !$item->isLink() ? @rmdir($item->getPathname()) : @unlink($item->getPathname());
                    }
                    @rmdir($folder);
                });
                $params = $_SERVER;
                $params['MAGE_REQUIRE_MAINTENANCE'] = null;
                $params['MAGE_DIRS'] = [];
                foreach (['var', 'cache', 'log', 'tmp', 'session', 'generated', 'code', 'metadata'] as $code) {
                    $params['MAGE_DIRS'][$code] = ['path' => $folder . '/' . $code];
                }
                return \Magento\Framework\App\Bootstrap::create($this->root, $params)->getObjectManager();
            } finally {
                while (ob_get_level() > $level) {
                    ob_end_clean();
                }
            }
        }

        public function create($type, array $arguments = [])
        {
            return $this->real()->create($type, $arguments);
        }

        public function get($type)
        {
            return $this->real()->get($type);
        }

        public function configure(array $configuration)
        {
            $this->real()->configure($configuration);
        }
    });
}
"#;

/// Encodes a default value for the resolver (`resolve_non_object_default`): `__json__:` carries
/// JSON, `__php__:` and `__obj__:` a `var_export()` expression, anything else is a PHP literal.
pub const ENCODE_DEFAULT_PHP: &str = r#"// Encodes a default value. `__php__:` and `__obj__:` carry a var_export() expression.
function holds_object($v): bool {
    if (is_array($v)) { foreach ($v as $i) if (holds_object($i)) return true; return false; }
    return is_object($v);
}
function holds_instance($v): bool {
    if (is_array($v)) { foreach ($v as $i) if (holds_instance($i)) return true; return false; }
    return is_object($v) && !($v instanceof UnitEnum);
}
function encode_default($v): string {
    if (holds_instance($v)) return '__obj__:' . var_export($v, true);
    if (holds_object($v)) return '__php__:' . var_export($v, true);
    if (is_array($v)) {
        $json = json_encode($v);
        return $json === false ? '__php__:' . var_export($v, true) : '__json__:' . $json;
    }
    if ((is_float($v) && !is_finite($v)) || $v === PHP_INT_MIN || (is_string($v) && str_contains($v, "\0"))) {
        return '__php__:' . var_export($v, true);
    }
    return var_export($v, true);
}
"#;
