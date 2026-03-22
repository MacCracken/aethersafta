# Contributing to aethersafta

Thank you for your interest in contributing to aethersafta, a real-time media
compositing engine. This document covers everything you need to get started.

aethersafta is licensed under **AGPL-3.0-only**. By submitting a contribution
you agree to license it under the same terms.

## Getting Started

### Prerequisites

- Rust toolchain (MSRV **1.89**, edition 2024) -- the repo pins the version via
  `rust-toolchain.toml`
- On Linux: `libpipewire-0.3-dev` (PipeWire development headers)

```sh
# Debian/Ubuntu
sudo apt install libpipewire-0.3-dev
```

### Clone and build

```sh
git clone https://github.com/MacCracken/aethersafta.git
cd aethersafta
cargo build
```

Verify everything works:

```sh
make check
```

## Development Workflow

Before submitting any change, run the full local check suite:

```sh
make check   # runs fmt, clippy, test, audit
```

### Feature flags

Enable features as needed during development:

| Flag | Purpose |
|------|---------|
| `pipewire` | PipeWire capture/output support |
| `hwaccel` | Hardware-accelerated encoding (via ai-hwaccel) |
| `openh264-enc` | OpenH264 software encoder |
| `vaapi` | VA-API hardware encoding (Linux) |
| `rtmp` | RTMP output sink |
| `srt` | SRT output sink |

Example:

```sh
cargo build --features pipewire,hwaccel
cargo test --features rtmp,srt
```

## Project Layout

```
src/
  audio/      Audio mixing and processing (dhvani mixer)
  encode/     Encoding pipeline (tarang wrappers)
  output/     Output sinks (file, MP4, RTMP, SRT)
  scene/      Scene graph and compositor
  source/     Capture sources (camera, screen, PipeWire)
  timing/     Frame clock and synchronisation
  lib.rs      Library root
  main.rs     CLI entry point (clap)
```

Key dependencies:

- **tarang** -- encoding
- **ranga** -- image processing
- **dhvani** -- audio
- **ai-hwaccel** -- hardware detection
- **tokio** -- async runtime
- **serde** -- serialisation
- **tracing** -- structured logging

## Commit Conventions

Use [Conventional Commits](https://www.conventionalcommits.org/) format:

```
<type>(<scope>): <description>
```

**Types:** `feat`, `fix`, `refactor`, `perf`, `test`, `docs`, `ci`, `chore`

**Scopes** map to source modules: `audio`, `encode`, `output`, `scene`,
`source`, `timing`, `cli`.

Examples:

```
feat(scene): add layer opacity blending
fix(audio): correct sample rate conversion in dhvani mixer
perf(encode): reduce copy in tarang pipeline flush
ci: add MSRV check to GitHub Actions
```

## PR Guidelines

- Keep PRs small and focused -- one logical change per PR.
- Every new feature or bug fix must include tests.
- All clippy warnings must be resolved (`cargo clippy -- -D warnings`).
- Code must be formatted with `rustfmt` (`cargo fmt`).
- Reference related issues in the PR description.
- Rebase on `main` before requesting review; avoid merge commits.

## Testing

Run the test suite:

```sh
cargo test                # unit + integration tests
cargo bench               # benchmarks (in benches/)
```

Coverage target is **85%+**. Check coverage locally with:

```sh
cargo llvm-cov --html     # generates target/llvm-cov/html/index.html
```

When adding new modules or public API surface, add corresponding tests in the
module or under `src/tests/`.

## Code Style

- Follow existing patterns in the codebase.
- Do **not** use `unsafe` without an accompanying `// SAFETY:` comment
  explaining the invariants.
- Use **thiserror** for library error types and **anyhow** for application-level
  error handling.
- Prefer explicit types on public APIs; elide within function bodies where the
  type is obvious.
- Use `tracing` macros (`tracing::info!`, `tracing::debug!`, etc.) instead of
  `println!` or `eprintln!`.
