# Nix builds

`flake.nix` exposes the Helix package and development shell. The implementation
lives in this directory. Legacy non-flake shells can use `nix-shell nix/shell.nix`.

Grammar sources are pinned in `grammar-sources.json`, keyed by grammar name.
Each entry contains the source declaration from `languages.toml` and its recursive
SHA-256 content hash. `grammars.nix` uses fixed-output `fetchFromGitHub` and
`fetchgit` derivations, so evaluating Helix does not download grammar sources.
Binary cache hits can therefore skip both source downloads and compilation.
A cache miss still fetches the source during the build.

After changing grammar revisions or the `use-grammars` selection, run from the
repository root (Python 3.11+ and Nix are required):

```sh
python3 nix/update-grammars.py
python3 nix/update-grammars.py --check
```

The updater reuses unchanged entries and deduplicates repositories shared by
multiple grammars. Only the update command uses `builtins.fetchTree` to obtain
content hashes. It replaces the lock file after every requested source succeeds;
`--check` never fetches or writes. CI checks that selected grammar declarations
match the lock before requesting a binary cache build.

The unavailable Cylc repository is excluded in `languages.toml`, alongside the
other disabled grammars. To restore it, update its source to an accessible
revision, remove the exclusion, and regenerate the lock.

The host configuration must lock a published Helix revision containing these
files before it can use this fetch strategy. The first CI build of that revision
populates the new derivations in Cachix; existing derivations use different
source dependencies.
