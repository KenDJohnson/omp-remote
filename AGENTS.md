# Repository guidance

## Nix-first development

- Treat `flake.nix` as the source of truth for the development and test environment.
- Add every required development or testing executable to the flake's default dev shell. Do not install tools ad hoc with `cargo install`, `rustup`, Homebrew, or `nix profile`.
- Keep `.envrc` on nix-direnv's `use flake` integration. Run repository commands that need project tools through the dev shell with `direnv exec . <command>`.
- Build and configure project workflows Nix-first. A command used by contributors or automation must work from the flake-provided environment.
- Use `flake-parts` to structure flake outputs. Do not add or use `flake-utils`.
- Keep Nix expressions small and direct. Format all Nix code with `direnv exec . alejandra .` before committing.

## Rust checks

- Keep dependencies shared at the workspace level when multiple crates use them.
- Run focused checks while iterating, then run `direnv exec . nix flake check` before committing changes that affect the workspace.
