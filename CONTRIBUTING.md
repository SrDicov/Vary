# Contributing to vary

Thanks for your interest! vary is a GPL-3.0 fork of [paru](https://github.com/Morganamilo/paru) ported to Void Linux/XBPS by [Dicov (SrDicov)](https://github.com/SrDicov).

## Development

```sh
# Run the test suite (68 tests; 4 ignored — they need a real Void Linux + xbps-query)
cargo test

# Build
cargo build --release

# Debug output
VARY_DEBUG=1 cargo run -- -Ss foo
```

- Toolchain: Rust 1.87+ (`rust-version` in `Cargo.toml`), edition 2021.
- Keep PRs focused: one feature or fix per PR.
- Update both `README.md` (English) and `README.es.md` (Spanish) when changing user-facing behavior.
- VUR repo index format changes must update [`docs/VURINFO.md`](docs/VURINFO.md) and bump `format_version` handling in `src/metadata.rs`.

## Reporting issues

Open an issue with:

- Void Linux variant (glibc/musl) and arch
- `vary` version (`vary -V`)
- Full command and output with `VARY_DEBUG=1`
