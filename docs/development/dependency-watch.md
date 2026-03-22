# Dependency watch

Upstream issues blocking clean builds or future cleanup.

## cros-libva 0.0.13 — VP9 struct mismatch

**Status**: Waiting on upstream (Google/ChromeOS)

**Issue**: `cros-libva` 0.0.13 is missing two fields (`seg_id_block_size`, `va_reserved8`) in
`VAEncPictureParameterBufferVP9`, causing build failures with libva >= 1.23.

**Workaround**: `[patch.crates-io]` pointing to `patches/cros-libva/` (vendored from tarang).
Every downstream crate enabling `vaapi` must carry this patch independently because
`[patch.crates-io]` does not propagate to consumers.

**Upstream repo**: https://github.com/chromeos/cros-libva

**Remove when**: `cros-libva` > 0.0.13 is published on crates.io with the fix. At that point:
1. Delete `patches/cros-libva/`
2. Remove `[patch.crates-io]` from `Cargo.toml`
3. Run `cargo update -p cros-libva`
