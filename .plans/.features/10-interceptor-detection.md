# 10: Interceptor Detection

- Category: Resolution
- Status: Planned
- Implementation Phase: 03-di-resolver
- Owner: Unassigned
- Feature ID: `interceptor-detection`
- Suggested Dependencies: 06-extract-result-type, 09-di-config-model

## Intent

Determine which classes need an interceptor generated, and build the `InterceptorSpec`
for each (including the list of public methods that need wrapping).

## Detection Rules

A class needs an interceptor if ALL of:
- Has one or more `<plugin type="X">` entries in merged di.xml (after preference resolution)
- Is NOT `final`
- Is NOT abstract
- Has at least one public non-final method

## Core State and Actions

```rust
pub fn detect_interceptors(
    class_map: &HashMap<String, ClassInfo>,
    config: &DiConfig,
) -> Vec<InterceptorSpec>

pub struct InterceptorSpec {
    pub fqcn: String,
    pub plugins: Vec<Plugin>,                        // sorted by sort_order
    pub public_methods: Vec<MethodSignature>,        // all public non-final methods
}
```

**Important:** preference resolution must be applied — if `di.xml` has
`<preference for="InterfaceX" type="ConcreteY">` and `ConcreteY` has plugins,
both `InterfaceX` and `ConcreteY` may appear in plugin `type=` attributes.

## Acceptance Criteria

- Interceptor list matches PHP compiler's INTERCEPTION operation output
- `final` classes are excluded
- Plugin sort order preserved in `InterceptorSpec.plugins`
- Classes with no public methods excluded (nothing to intercept)
