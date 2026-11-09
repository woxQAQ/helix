## Using pickers

Helix has a variety of pickers, which are interactive windows used to select various kinds of items. These include a file picker, global search picker, and more. Most pickers are accessed via keybindings in [space mode](./keymap.md#space-mode). Pickers have their own [keymap](./keymap.md#picker) for navigation.

### Filtering Picker Results

Most pickers perform fuzzy matching using [fzf syntax](https://github.com/junegunn/fzf?tab=readme-ov-file#search-syntax). Two exceptions are the global search picker, which uses regex, and the workspace symbol picker, which passes search terms to the language server. Note that OR operations (`|`) are not currently supported.

If a picker shows multiple columns, you may apply the filter to a specific column by prefixing the column name with `%`. Column names can be shortened to any prefix, so `%p`, `%pa` or `%pat` all mean the same as `%path`. For example, a query of `helix %p .toml !lang` in the global search picker searches for the term "helix" within files with paths ending in ".toml" but not including "lang".

You can insert the contents of a [register](./registers.md) using `Ctrl-r` followed by a register name. For example, one could insert the currently selected text using `Ctrl-r`-`.`, or the directory of the current file using `Ctrl-r`-`%` followed by `Ctrl-w` to remove the last path section. The global search picker will use the contents of the [search register](./registers.md#default-registers) if you press `Enter` without typing a filter. For example, pressing `*`-`Space-/`-`Enter` will start a global search for the currently selected text.

### File explorer

`Space-e` toggles a persistent file explorer sidebar rooted at the workspace. `Space-E` focuses the sidebar, opening it if necessary. Opening a file keeps the sidebar visible and returns focus to the document. The current file is revealed when the sidebar first opens.

The sidebar shares the ignore and directory-flattening options in [`[editor.file-explorer]`](./editor.md#editorfile-explorer-section). Set `sidebar-width` there to change its width (30 columns by default, including the separator). It temporarily hides when the terminal is too small to leave room for both the tree and the document.

The sidebar displays Nerd Fonts 3 Material Design Icons (`nf-md-*`) by default. Configure the terminal to use a Nerd Font (preferably a Mono variant). Icons are defined by an embedded data-driven theme in `runtime/icons.toml`, generated from the complete Zed Material Icon Theme file-name, suffix, and directory associations. The default contains over 2,100 file-name associations, 1,300 suffixes, and 4,600 directory names. Upstream SVG icon IDs are adapted to NF Material glyphs; IDs without an adapted glyph retain their associations and use generic file or folder icons. File matching checks the full file name, then progressively shorter compound suffixes (for example, `stories.tsx` before `tsx`), then the name before the first dot (for example, `Dockerfile` in `Dockerfile.prod`). Helix's language detection supplies a fallback, including custom file types from `languages.toml`, followed by the default file icon. File and directory name associations are ASCII case-insensitive.

Directory names such as `src`, `docs`, `tests`, and `node_modules` have their own collapsed and expanded icons. The root heading uses the expanded icon. Missing named-directory states fall back to the generic folder icon for that state. To disable the icons:

```toml
[editor.file-explorer]
icons = false
```

Create `icons.toml` in the Helix configuration directory (next to `config.toml`) to override the built-in icon theme. Only supplied entries are replaced; other built-in associations and directory states are retained. Reopen the sidebar to load changes. For example:

```toml
# Define a glyph once, then reference its ID in associations.
[file_icons]
project = "\U000F0D09" # nf-md-folder_pound
project-open = "\U000F0D0A" # nf-md-folder_pound_outline

[file_stems]
"project.json" = "project"

[file_suffixes]
"stories.tsx" = "react"

[language_icons]
rust = "rust"

[named_directory_icons.my-project]
collapsed = "project"
expanded = "project-open"
```

`file_icons` maps icon IDs to single glyphs occupying one terminal column. `file_stems` maps specific file names, `file_suffixes` maps suffixes without a leading dot, and `language_icons` maps Helix language IDs to icon IDs. `directory_icons` sets the generic `collapsed` and `expanded` icon IDs; `named_directory_icons` provides the same pair for individual directory names. Icon IDs are case-sensitive. See `runtime/icons.toml` for built-in IDs and associations, and `contrib/icon-theme/README.md` for generation and adaptation details. An invalid overlay, including unknown icon IDs or invalid glyphs, is logged and falls back to the complete built-in theme.

Git changes color file names and icons. Modified files and `M` markers default to the theme's `warning` foreground (typically yellow); themes can override this with `"ui.file-explorer.git.modified"`. Other states use the theme's `diff` foregrounds. Directory names, folder icons, and the root heading retain their normal colors, with descendant changes summarized in the status badge. Selection backgrounds remain visible. The status column shows `M` for modified, `A` for staged additions, `?` for untracked, `D` for deleted, `R` for renamed, and `!` for conflicts. Both staged and working-tree changes are included, including deleted files that no longer appear in the tree. Clean files have no badge. Ignored files and directories use `ui.file-explorer.ignored`, falling back to `ui.text.inactive`; this also applies to descendants of ignored directories. Ignore styling reads `.gitignore`, `.ignore`, Git's global excludes and `.git/info/exclude`, and Helix ignore files independently of the options that hide entries. Ignored entries remain navigable, and selection backgrounds remain visible.

Directory badges retain every distinct descendant state, so a folder containing modified, staged-new, and untracked files shows `MA?`. The root heading aggregates the entire tree. Badges are deduplicated and ordered consistently as `!MRDA?`, with each marker keeping its own color. Each row reserves enough space for its badges; very narrow rows omit the file icon when needed to keep names visible.

Git status is loaded in the background when the sidebar opens and when `r` refreshes it. A refresh preserves expanded directories and selection. Outside a Git repository the sidebar works without Git badges.

| Key | Action |
| --- | --- |
| `j` / `k`, arrows | Move down / up |
| `l` / Right | Expand a directory, enter an expanded directory, or open a file |
| `h` / Left | Collapse a directory or select its parent |
| `Enter` | Toggle a directory or open a file |
| `Ctrl-v` / `Ctrl-s` | Open a file in a vertical / horizontal split |
| `g` / `G`, Home / End | Select the first / last entry |
| PageUp / PageDown, `Ctrl-u` / `Ctrl-d` | Move by a page |
| `r` | Refresh the tree and Git status, preserving expanded directories |
| `Esc` / Tab | Return focus to the document |
| `q` | Close the sidebar |

Opening a file inspects its content first. Binary files such as PNG images and PDFs open in the system's default application, leaving the document and split layout unchanged. Text files open in Helix, including UTF-16/32 files with a byte-order mark. This applies to mouse clicks, `Enter`, `l`, and split shortcuts.

Click an entry to toggle its directory or open its file. The mouse wheel moves the selection. Clicking the document returns focus to it. Command prompts (`:`), space-mode bindings, and window commands (`Ctrl-w`) remain accessible from the sidebar.

`Space-.` opens the directory picker at the current buffer's directory. The `file_explorer` command also remains available for a workspace-rooted picker and can be assigned a custom binding.
