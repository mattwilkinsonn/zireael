# License

This repository is dual-licensed under your choice of:

- **MIT License** — see [`LICENSE-MIT`](./LICENSE-MIT)
- **Apache License, Version 2.0** — see [`LICENSE-APACHE`](./LICENSE-APACHE)

You may use the code in this repository under the terms of either license.
The dual-license layout follows the Rust ecosystem convention (matching
cargo, tokio, serde, ripgrep, and most other widely-used crates).

## Per-tool exceptions

`tools/akiflow-cli/` is a fork of upstream
[`code-yeongyu/akiflow-cli`](https://github.com/code-yeongyu/akiflow-cli),
which is **MIT-licensed only**. The fork retains the upstream MIT license at
[`tools/akiflow-cli/LICENSE`](./tools/akiflow-cli/LICENSE) — Apache-2.0 does
not apply to that subdirectory.

`Formula/*.rb` are Homebrew formula stubs and carry no additional
copyright — they describe how to install the binaries built by this repo
under the dual-license terms above.
