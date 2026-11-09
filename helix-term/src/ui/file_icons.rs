use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, LazyLock},
};

use anyhow::{bail, Context};
use helix_core::{syntax::Loader, unicode::width::UnicodeWidthStr};
use serde::Deserialize;

static DEFAULT_THEME: LazyLock<Arc<IconTheme>> = LazyLock::new(|| {
    let theme = IconTheme::parse(include_str!("../../../runtime/icons.toml"))
        .expect("invalid built-in icon theme");
    theme
        .validate()
        .expect("invalid built-in icon associations");
    Arc::new(theme)
});

/// Associations resolve to icon IDs so glyph overrides apply to every use of an icon.
#[derive(Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct IconTheme {
    file_icons: HashMap<String, String>,
    file_stems: HashMap<String, String>,
    file_suffixes: HashMap<String, String>,
    language_icons: HashMap<String, String>,
    directory_icons: DirectoryIcons,
    named_directory_icons: HashMap<String, DirectoryIcons>,
}

#[derive(Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct DirectoryIcons {
    collapsed: Option<String>,
    expanded: Option<String>,
}

impl DirectoryIcons {
    fn get(&self, expanded: bool) -> Option<&str> {
        if expanded {
            self.expanded.as_deref()
        } else {
            self.collapsed.as_deref()
        }
    }

    fn extend(&mut self, overrides: Self) {
        if overrides.collapsed.is_some() {
            self.collapsed = overrides.collapsed;
        }
        if overrides.expanded.is_some() {
            self.expanded = overrides.expanded;
        }
    }
}

impl IconTheme {
    /// Load a user overlay once when the sidebar opens. Invalid overlays leave defaults intact.
    pub(super) fn load(path: &Path) -> Arc<Self> {
        let source = match std::fs::read_to_string(path) {
            Ok(source) => source,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return DEFAULT_THEME.clone(),
            Err(err) => {
                log::warn!("failed to read icon theme {}: {err}", path.display());
                return DEFAULT_THEME.clone();
            }
        };
        match Self::parse(&source).and_then(|overrides| {
            let mut theme = (**DEFAULT_THEME).clone();
            theme.file_icons.extend(overrides.file_icons);
            theme.file_stems.extend(overrides.file_stems);
            theme.file_suffixes.extend(overrides.file_suffixes);
            theme.language_icons.extend(overrides.language_icons);
            theme.directory_icons.extend(overrides.directory_icons);
            for (name, icons) in overrides.named_directory_icons {
                theme
                    .named_directory_icons
                    .entry(name)
                    .or_default()
                    .extend(icons);
            }
            theme.validate()?;
            Ok(theme)
        }) {
            Ok(theme) => Arc::new(theme),
            Err(err) => {
                log::warn!("failed to load icon theme {}: {err:#}", path.display());
                DEFAULT_THEME.clone()
            }
        }
    }

    fn parse(source: &str) -> anyhow::Result<Self> {
        fn normalize<T>(map: &mut HashMap<String, T>) -> anyhow::Result<()> {
            let mut normalized = HashMap::new();
            for (key, value) in map.drain() {
                let key = key.to_ascii_lowercase();
                if normalized.insert(key.clone(), value).is_some() {
                    bail!("duplicate icon association {key:?} (names are case-insensitive)");
                }
            }
            *map = normalized;
            Ok(())
        }
        let mut theme: Self = toml::from_str(source).context("invalid icon theme TOML")?;
        normalize(&mut theme.file_stems)?;
        normalize(&mut theme.file_suffixes)?;
        normalize(&mut theme.named_directory_icons)?;
        Ok(theme)
    }

    fn validate(&self) -> anyhow::Result<()> {
        for (id, glyph) in &self.file_icons {
            if glyph.chars().count() != 1
                || glyph.width() != 1
                || glyph.chars().any(char::is_control)
            {
                bail!("icon {id:?} must be a single glyph occupying one terminal column");
            }
        }
        let check = |id: &str| -> anyhow::Result<()> {
            if !self.file_icons.contains_key(id) {
                bail!("unknown icon ID {id:?}");
            }
            Ok(())
        };
        check("default")?;
        for id in self
            .file_stems
            .values()
            .chain(self.file_suffixes.values())
            .chain(self.language_icons.values())
        {
            check(id)?;
        }
        for icons in
            std::iter::once(&self.directory_icons).chain(self.named_directory_icons.values())
        {
            for id in [&icons.collapsed, &icons.expanded].into_iter().flatten() {
                check(id)?;
            }
        }
        if self.directory_icons.collapsed.is_none() || self.directory_icons.expanded.is_none() {
            bail!("generic directory icons must define collapsed and expanded states");
        }
        Ok(())
    }

    pub(super) fn directory_icon(&self, path: &Path, expanded: bool) -> &str {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let id = self
            .named_directory_icons
            .get(&name)
            .and_then(|icons| icons.get(expanded))
            .or_else(|| self.directory_icons.get(expanded))
            .expect("validated generic directory icon");
        &self.file_icons[id]
    }

    pub(super) fn file_icon(&self, path: &Path, loader: &Loader) -> &str {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let lookup = |candidate: &str| {
            self.file_stems
                .get(candidate)
                .or_else(|| self.file_suffixes.get(candidate))
        };
        // Try the full name, then successively shorter compound suffixes.
        // This lets a rule for stories.tsx take precedence over tsx.
        let mut candidate = name.as_str();
        loop {
            if let Some(id) = lookup(candidate) {
                return &self.file_icons[id];
            }
            match candidate.split_once('.') {
                Some((_, suffix)) => candidate = suffix,
                None => break,
            }
        }
        // Preserve named-file associations for variants such as Dockerfile.prod.
        if let Some((stem, _)) = name.split_once('.') {
            if let Some(id) = self.file_stems.get(stem) {
                return &self.file_icons[id];
            }
        }
        // Language detection covers custom file-types from languages.toml.
        if let Some(language) = loader.language_for_filename(path) {
            let language = loader.language(language);
            if let Some(id) = self
                .language_icons
                .get(language.config().language_id.as_str())
            {
                return &self.file_icons[id];
            }
        }
        &self.file_icons["default"]
    }
}
