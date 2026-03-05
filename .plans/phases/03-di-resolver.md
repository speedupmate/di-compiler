# Phase 03: DI Resolver

## Purpose

Build the `di-resolver` crate. Given `ClassInfo` map + merged `DiConfig`, determine which
classes need interceptors, factories, and proxies, and resolve all constructor arguments
to Magento's `_i_`/`_v_`/`_ins_` notation.

No I/O in this crate — pure logic.

## Gate To Enter

Phase 01 (ClassInfo extraction) and Phase 02 (DiConfig merge) complete.

## Gate To Complete

- `ResolvedGraph` matches PHP `ArgumentsResolver` output for 100-class sample
- Interceptor candidate list matches PHP compiler's interception pass output
- Factory and Proxy candidate lists correct

## Features In This Phase

| Feature | Deps |
|---------|------|
| [10-interceptor-detection](../.features/10-interceptor-detection.md) | 06, 09 |
| [11-factory-detection](../.features/11-factory-detection.md) | 06, 09 |
| [12-proxy-detection](../.features/12-proxy-detection.md) | 06, 09 |
| [13-arguments-resolver](../.features/13-arguments-resolver.md) | 09, 06 |

## Key Resolution Rules

**Interceptor:** has `<plugin type="X">` in di.xml AND class is not `final` AND not abstract
**Factory:** constructor param type hint ends in `Factory` AND that class doesn't exist on disk
**Proxy:** constructor param ends in `\Proxy` OR di.xml `xsi:type="object"` with `\Proxy` suffix

**Argument notation:**
- Typed required param → `['_i_' => 'FQN']` (shared) or `['_ins_' => 'FQN']` (non-shared)
- Optional → `['_v_' => default]` or `['_vn_' => true]` for null
- di.xml `<argument>` overrides reflection result

## Tickets In This Phase

TKT-013 through TKT-015
