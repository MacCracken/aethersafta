# Security Policy

## Supported Versions

Aethersafta is pre-1.0 software. Only the latest release is supported with
security updates.

| Version | Supported |
| ------- | --------- |
| 0.21.x (latest) | Yes |
| < 0.21.0 | No |

## Reporting a Vulnerability

**Do not open public issues for security vulnerabilities.**

Send reports to **security@maccracken.dev** with:

- A description of the vulnerability and its potential impact.
- Steps to reproduce or a proof of concept, if available.
- The version of aethersafta affected.

**Response timeline:**

- **72 hours** — initial acknowledgement of your report.
- **90 days** — coordinated disclosure deadline. We aim to release a fix before
  this window closes. If more time is needed, we will negotiate an extension
  with the reporter.

We will credit reporters in the release notes unless they prefer to remain
anonymous.

## Threat Model

Aethersafta is a real-time media compositing engine that processes untrusted
media inputs, manages GPU and shared memory, and streams output over the
network. The following areas are considered security-sensitive.

### Scene Graph Injection

Scene graph configurations can reference external resources (e.g.,
`ImageSource` file paths). A maliciously crafted scene graph could attempt to
read arbitrary files from the host filesystem. All resource paths must be
validated and restricted to an allowed set of directories.

### GPU and Shared Memory

The compositor allocates shared memory buffers for frame data and uses SIMD
routines (via `ranga`) for image processing. Out-of-bounds reads or writes in
these code paths could lead to information disclosure or memory corruption.

### Streaming Credentials

RTMP and SRT streaming configurations may contain authentication credentials.
These values must never be written to logs, error messages, or diagnostic
output.

### FFI Safety

Aethersafta relies on unsafe FFI boundaries in several crates:

- **tarang** — video/audio encoding.
- **ranga** — SIMD image processing.
- **dhvani** — PipeWire audio capture and playback.

Memory safety violations in any of these boundaries could compromise the entire
process.

### PipeWire Capture

Audio capture through `dhvani` connects to the system PipeWire daemon and may
have access to audio streams from other applications. Deployments should
restrict PipeWire permissions to limit the capture scope.

## Security Practices

The following measures are enforced in CI for every pull request:

- **cargo-audit** — checks dependencies against the RustSec Advisory Database.
- **cargo-deny** — enforces supply-chain policies (license compliance,
  duplicate dependencies, and known vulnerabilities via `deny.toml`).
- **Clippy warnings as errors** — all Clippy lints are treated as hard errors.
- **Unsafe documentation** — every `unsafe` block must include a `// SAFETY:`
  comment explaining why the usage is sound.

## License

This project is licensed under AGPL-3.0-only. See [LICENSE](LICENSE) for
details.
