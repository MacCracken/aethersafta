# ADR-002: SIMD Delegation to ranga

**Status**: Accepted
**Date**: 2026-03-21 (v0.21.3)

## Context

The compositor's hot paths — alpha blending, color conversion, pixel format interchange — benefit significantly from SIMD acceleration. The question is whether to implement SIMD directly in aethersafta or delegate to the `ranga` crate.

## Decision

Delegate all SIMD-accelerated image operations to **ranga**. Aethersafta contains no `unsafe` blocks or inline SIMD intrinsics. The compositor calls ranga's blend, convert, and transform APIs which handle architecture-specific dispatch internally.

## Rationale

1. **Own the stack, not the SIMD.** ranga is an AGNOS crate maintained by the same team. Centralising SIMD in ranga means one place to audit `unsafe`, one place to add AVX2/NEON, and all consumers (aethersafta, tazama, etc.) benefit.

2. **Zero unsafe in aethersafta.** As of v0.24.3, aethersafta has zero `unsafe` blocks in `src/`. All memory-unsafe operations live in ranga behind safe APIs. This simplifies auditing and reduces the attack surface of the compositor.

3. **Portable SIMD future.** When `std::simd` stabilises, ranga can migrate from manual SSE2/AVX2 intrinsics to portable SIMD without any changes to aethersafta.

4. **Measurable boundary.** Benchmark comparisons between aethersafta versions directly measure ranga's blend/convert performance. Regressions (like the v0.21.3 source-layer regression from ranga 0.21.4) are visible and attributable.

## Trade-offs

- **Dependency coupling.** aethersafta's blend performance is entirely determined by ranga. A ranga regression becomes an aethersafta regression (observed in v0.21.3 benchmarks: 5x source-layer slowdown from ranga 0.21.4 blend path change).

- **Indirection cost.** Function call overhead through ranga vs inline intrinsics. In practice, ranga's functions are `#[inline]` and the compiler eliminates this overhead.

- **Format conversion overhead.** ranga's resize requires RGBA8, not ARGB8. Aethersafta converts ARGB→RGBA→resize→RGBA→ARGB. This round-trip adds measurable overhead for scaled layers.

## Alternatives Considered

- **Inline SSE2 in compositor**: v0.20.3 had hand-written SSE2 alpha blending (9.4x speedup). Migrated to ranga in v0.21.3 for centralisation. The SSE2 code was correct but duplicated logic that ranga now owns.
- **`std::simd` (portable SIMD)**: Not yet stable. When it is, ranga will adopt it — aethersafta gets the benefit for free.
