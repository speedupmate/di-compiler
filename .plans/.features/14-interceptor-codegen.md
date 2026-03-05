# 14: Interceptor Code Generator

- Category: Code Generation
- Status: Planned
- Implementation Phase: 04-code-generator
- Owner: Unassigned
- Feature ID: `interceptor-codegen`
- Suggested Dependencies: 10-interceptor-detection, 13-arguments-resolver

## Intent

Generate `*Interceptor.php` files in `generated/code/` for each `InterceptorSpec`.
Output must be byte-for-byte identical to PHP INTERCEPTION operation output.

## PHP Template

```php
<?php
namespace {Namespace};

/**
 * Interceptor class for @see \{OriginalFQCN}
 */
class Interceptor extends \{OriginalFQCN} implements \Magento\Framework\Interception\InterceptorInterface
{
    use \Magento\Framework\Interception\Interceptor;

    public function __construct({constructor_params})
    {
        $this->___init();
        parent::__construct({parent_args});
    }

{method_wrappers}
}
```

## Method Wrapper Template (per public method)

```php
    public function {name}({params})
    {
        $pluginInfo = $this->pluginList->getNext($this->subjectType, '{name}');
        return $pluginInfo ? $this->___callPlugins('{name}', func_get_args(), $pluginInfo) : parent::{name}({args});
    }
```

## Core State and Actions

```rust
pub fn generate_interceptor(spec: &InterceptorSpec, class_info: &ClassInfo) -> String
```

## Acceptance Criteria

- Output matches PHP-generated interceptors byte-for-byte (verified by TKT-023)
- Namespace derived from target class FQN (same namespace, class name = Interceptor)
- All public non-final methods wrapped
- Constructor calls `___init()` then `parent::__construct`
- File written to `generated/code/{Vendor}/{Module}/{Path}/Interceptor.php`
