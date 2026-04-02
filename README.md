# nu_plugin_hebuni

Hebrew-specific substrate and recomposition helpers for Nushell.

This plugin is intentionally narrow: it handles pointed Hebrew scalar stripping and recomposition logic that is specific to the Hebrew pipeline. It is not a generic Unicode wrapper.

## Commands

- `hebuni scalar-strip`: break a pointed Hebrew surface form into consonants, NFC chars, and stripped scalars.
- `hebuni recompose`: rebuild a canonical NFC string from a kept subset of NFC scalar positions.

## Scope

- Uses standard Rust unicode primitives.
- Applies Hebrew-specific structure and selection rules.
- Keeps the public command surface small so TE2 can use it as a focused helper.
