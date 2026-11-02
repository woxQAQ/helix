use std::path::Path;

use helix_core::syntax::Loader;

// Glyphs use Nerd Fonts 3 names from https://www.nerdfonts.com/cheat-sheet.
pub(super) const FOLDER: &str = "\u{f07b}"; // fa-folder
pub(super) const FOLDER_OPEN: &str = "\u{f07c}"; // fa-folder_open
const FILE: &str = "\u{f016}"; // fa-file_o

pub(super) fn file_icon(path: &Path, loader: &Loader) -> &'static str {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    match name {
        "Cargo.toml" | "Cargo.lock" | "rust-toolchain" | "rust-toolchain.toml" => {
            return "\u{e7a8}"
        }
        "package.json" | "package-lock.json" | "pnpm-lock.yaml" | "yarn.lock" | "bun.lock"
        | "bun.lockb" => return "\u{e71e}",
        ".gitignore" | ".gitattributes" | ".gitmodules" | ".gitconfig" => return "\u{e702}",
        "Dockerfile"
        | "Containerfile"
        | ".dockerignore"
        | "compose.yaml"
        | "compose.yml"
        | "docker-compose.yaml"
        | "docker-compose.yml" => return "\u{e7b0}",
        ".editorconfig" | ".env" => return "\u{e615}",
        _ => {}
    }
    if let Some(language) = loader.language_for_filename(path) {
        let icon = match loader.language(language).config().language_id.as_str() {
            "rust" => "\u{e7a8}",
            "go" | "gomod" | "gowork" | "gotmpl" => "\u{e724}",
            "python" | "python3" | "cython" => "\u{e73c}",
            "nix" => "\u{f313}",
            "javascript" => "\u{e781}",
            "typescript" => "\u{e628}",
            "jsx" | "tsx" => "\u{e7ba}",
            "json" | "jsonc" | "json5" | "json-ld" => "\u{e60b}",
            "html" | "htmldjango" => "\u{e736}",
            "css" => "\u{e749}",
            "scss" | "sass" => "\u{e74b}",
            "less" => "\u{e758}",
            "vue" => "\u{e8dc}",
            "svelte" => "\u{e8b7}",
            "markdown" | "mdx" => "\u{e73e}",
            "bash" | "sh" | "fish" | "nu" | "powershell" => "\u{ea85}",
            "lua" => "\u{e620}",
            "c" => "\u{e649}",
            "cpp" => "\u{e646}",
            "c-sharp" | "csharp" => "\u{e648}",
            "java" => "\u{e738}",
            "kotlin" => "\u{e81b}",
            "swift" => "\u{e755}",
            "ruby" => "\u{e739}",
            "php" => "\u{e73d}",
            "elixir" => "\u{e7cd}",
            "erlang" => "\u{e7b1}",
            "haskell" => "\u{e777}",
            "scala" => "\u{e737}",
            "zig" => "\u{e6a9}",
            "sql" => "\u{f1c0}",
            "xml" => "\u{e8ea}",
            "dockerfile" => "\u{e7b0}",
            "git-commit" | "git-config" | "git-rebase" | "git-ignore" => "\u{e702}",
            "toml" | "yaml" | "ini" => "\u{e615}",
            _ => FILE,
        };
        if icon != FILE {
            return icon;
        }
    }
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "ico" | "bmp" | "avif" => "\u{f1c5}",
        "zip" | "gz" | "bz2" | "xz" | "zst" | "tar" | "7z" | "rar" => "\u{f1c6}",
        "mp3" | "wav" | "ogg" | "flac" | "m4a" => "\u{f1c7}",
        "mp4" | "mkv" | "webm" | "mov" | "avi" => "\u{f1c8}",
        "pdf" => "\u{f1c1}",
        "db" | "sqlite" | "sqlite3" => "\u{f1c0}",
        "lock" => "\u{f023}",
        _ => FILE,
    }
}
