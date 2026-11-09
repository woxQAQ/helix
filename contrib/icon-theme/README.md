# NF Material icon theme generation

`runtime/icons.toml` is generated from the complete association tables in the Zed Material Icon Theme extension. The renderer uses Nerd Fonts 3 Material Design glyphs; SVG files and font binaries are not distributed.

## Inputs

- Zed Material Icon Theme manifest, pinned to commit `95cc434119c669e0ff65a6bccfd026ec70e16b63`: <https://github.com/zed-extensions/material-icon-theme/tree/95cc434119c669e0ff65a6bccfd026ec70e16b63>. Its JSON is generated from VS Code Material Icon Theme 5.39.0.
- Nerd Fonts `v3.4.0` glyph names and Unicode code points: <https://github.com/ryanoasis/nerd-fonts/blob/v3.4.0/glyphnames.json>. The generator also checks the input's SHA-256 digest.
- `material-nf.json`, the local adapter. `glyphs` groups upstream file icon IDs by NF glyph name. `folder_glyphs` groups folder categories by NF folder glyph. The remaining tables supplement upstream associations and provide Helix language IDs.

## Generation

Run from the repository root with Bun; no npm packages are required:

```nu
bun contrib/icon-theme/generate.mjs
```

Verify that committed artifacts match their inputs:

```nu
bun contrib/icon-theme/generate.mjs --check
```

Both commands fetch the pinned upstream manifest, NF glyph data, and license notices. Generation writes `runtime/icons.toml` and `runtime/icons.LICENSE`; check mode compares their content without writing. Normal editor startup does not access the network: the generated TOML is embedded in the binary.

To update upstream data, change the pinned revision or NF version and checksum in `generate.mjs`, regenerate, inspect the reported fallback counts, and run the file explorer rendering scenarios. To adjust glyph choices, edit `material-nf.json` and regenerate. Changes to generated TOML will be replaced on the next generation.

## Association coverage and glyph adaptation

The generator preserves all upstream file-name, suffix, and directory-name associations. It normalizes ASCII case and rejects conflicting case variants. The upstream's 9,416 file-name entries collapse to 2,148 unique names; its 1,381 suffix and 4,659 directory associations are retained. Local supplements produce 2,151 file-name associations, 1,381 suffixes, and 4,660 named directories.

Each upstream icon ID is resolved through the adapter first. Otherwise, file IDs are matched against NF Material icon names and `language_*` names, including base names of variants. Folder categories use adapter groups or matching NF folder glyphs; an outline variant represents expansion where available. Categories without an outline can use the same glyph for both states. Generic directories use `folder` / `folder_open`.

The current output has 1,219 icon ID definitions backed by 136 distinct NF glyphs. An ID definition is not a unique glyph. Material Icon Theme contains many brand and framework SVGs without a corresponding NF Material glyph. The current adapter uses a generic file glyph for 400 file IDs and a generic folder pair for 137 directory categories. These associations are preserved, even when their visual fallback is shared. The generated header reports fallback counts, and generic file mappings are marked inline. Adding a dedicated adapter entry improves all associations using that ID without changing Rust.

The generated TOML has 9,489 lines. Upstream uses expanded JSON objects and repeats file names in multiple case forms; this output deduplicates those names and emits directory state pairs on one line.

`runtime/icons.LICENSE` records the extension's Apache-2.0 license, the upstream Material Icon Theme MIT license, and the Nerd Fonts licensing notice. No SVG artwork or font binary is copied into this repository.
