# 15: Factory Code Generator

- Category: Code Generation
- Status: Planned
- Implementation Phase: 04-code-generator
- Owner: Unassigned
- Feature ID: `factory-codegen`
- Suggested Dependencies: 11-factory-detection

## Intent

Generate `*Factory.php` files in `generated/code/` for each `FactorySpec`.

## PHP Template

```php
<?php
namespace {Namespace};

/**
 * Factory class for @see \{TargetFQCN}
 */
class {ClassName}Factory
{
    /**
     * @var \Magento\Framework\ObjectManagerInterface
     */
    protected $_objectManager = null;

    /**
     * @var string
     */
    protected $_instanceName = null;

    public function __construct(
        \Magento\Framework\ObjectManagerInterface $objectManager,
        $instanceName = '\\{TargetFQCN}'
    ) {
        $this->_objectManager = $objectManager;
        $this->_instanceName = $instanceName;
    }

    public function create(array $data = [])
    {
        return $this->_objectManager->create($this->_instanceName, $data);
    }
}
```

## Core State and Actions

```rust
pub fn generate_factory(spec: &FactorySpec) -> String
```

## Acceptance Criteria

- Output matches PHP APPLICATION_CODE_GENERATOR factory output byte-for-byte
- Written to `generated/code/{FQN path}/Factory.php`
