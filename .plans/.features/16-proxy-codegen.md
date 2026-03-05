# 16: Proxy Code Generator

- Category: Code Generation
- Status: Planned
- Implementation Phase: 04-code-generator
- Owner: Unassigned
- Feature ID: `proxy-codegen`
- Suggested Dependencies: 12-proxy-detection

## Intent

Generate `*Proxy.php` files in `generated/code/` for each `ProxySpec`.
Proxies implement lazy initialization — the real object is created on first method call.

## PHP Template (simplified)

```php
<?php
namespace {Namespace};

/**
 * Proxy class for @see \{TargetFQCN}
 */
class Proxy extends \{TargetFQCN} implements \Magento\Framework\ObjectManager\NoninterceptableInterface
{
    // ... ObjectManager injection, __sleep, __wakeup, proxied method wrappers
}
```

## Core State and Actions

```rust
pub fn generate_proxy(spec: &ProxySpec, class_info: &ClassInfo) -> String
```

## Acceptance Criteria

- Output matches PHP PROXY_GENERATOR output byte-for-byte
- All public methods proxied (delegate to `_getSubject()->method()`)
- `__sleep`/`__wakeup` included
- Written to `generated/code/{FQN path}/Proxy.php`
