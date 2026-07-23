# Contributing

Contributions to OverCrow are welcome. By submitting a contribution, you agree
to license it under `AGPL-3.0-only` and confirm that you are authorized to do
so. Do not submit third-party code, assets, or data without documenting their
origin and compatible license.

Before opening a pull request:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets --locked
```

Run the relevant shell, packaging, or KWin checks listed in `AGENTS.md` when
those areas change. Keep changes focused, preserve the external-window
anti-cheat safety model, and describe any real-machine validation separately.

Contributors retain copyright in their work. This project currently requires
neither copyright assignment nor a contributor license agreement.
