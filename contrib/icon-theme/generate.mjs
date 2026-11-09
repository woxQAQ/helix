#!/usr/bin/env bun
// Generate NF associations from the pinned Zed Material Icon Theme manifest.
import { readFile, writeFile } from "node:fs/promises";
import { basename } from "node:path";

const root = new URL("../../", import.meta.url);
const materialRevision = "95cc434119c669e0ff65a6bccfd026ec70e16b63";
const materialRoot = `https://raw.githubusercontent.com/zed-extensions/material-icon-theme/${materialRevision}/`;
const nerdFontsRoot = "https://raw.githubusercontent.com/ryanoasis/nerd-fonts/v3.4.0/";
const glyphDigest = "e2d10d23f5bff0bd6f0676e9b01d9789fcdc656de7b498a2955c27716ea4439c";
const checkOnly = process.argv.slice(2).includes("--check");
if (process.argv.slice(2).some((arg) => arg !== "--check")) {
  throw new Error("Usage: bun contrib/icon-theme/generate.mjs [--check]");
}

async function download(url) {
  const response = await fetch(url);
  if (!response.ok) throw new Error(`${url}: HTTP ${response.status}`);
  return response.text();
}

const [materialSource, glyphSource, bridgeSource, zedLicense, materialLicense, nfLicense] = await Promise.all([
  download(`${materialRoot}icon_themes/material-icon-theme.json`),
  download(`${nerdFontsRoot}glyphnames.json`),
  readFile(new URL("material-nf.json", import.meta.url), "utf8"),
  download(`${materialRoot}LICENSE`),
  download("https://raw.githubusercontent.com/material-extensions/vscode-material-icon-theme/v5.39.0/LICENSE"),
  download(`${nerdFontsRoot}LICENSE`),
]);
const digest = new Bun.CryptoHasher("sha256").update(glyphSource).digest("hex");
if (digest !== glyphDigest) throw new Error("Nerd Fonts glyph data checksum mismatch");
const source = JSON.parse(materialSource).themes[0];
const glyphs = JSON.parse(glyphSource);
const bridge = JSON.parse(bridgeSource);
const glyphOverrides = new Map();
const folderOverrides = new Map();

function registerGroups(groups, target) {
  for (const [glyph, ids] of Object.entries(groups)) {
    if (!glyph.startsWith("md-") || !glyphs[glyph]) throw new Error(`Unknown Material NF glyph: ${glyph}`);
    for (const id of ids) {
      if (target.has(id)) throw new Error(`Duplicate NF bridge entry: ${id}`);
      target.set(id, glyph);
    }
  }
}
registerGroups(bridge.glyphs, glyphOverrides);
registerGroups(bridge.folder_glyphs, folderOverrides);

const asciiLower = (name) => name.replace(/[A-Z]/g, (letter) => letter.toLowerCase());
function normalize(entries) {
  const result = new Map();
  for (const [name, id] of Object.entries(entries)) {
    const key = asciiLower(name);
    if (result.has(key) && JSON.stringify(result.get(key)) !== JSON.stringify(id)) throw new Error(`Conflicting case variants: ${name}`);
    result.set(key, id);
  }
  return Object.fromEntries(result);
}

const fileIcons = new Map();
const fallbackFiles = new Set();
const fallbackFolders = new Set();
function fileGlyph(id) {
  if (glyphOverrides.has(id)) return glyphOverrides.get(id);
  let name = id.replace(/_light$/, "");
  while (name) {
    const normalized = name.replaceAll("-", "_");
    for (const key of [`md-${normalized}`, `md-language_${normalized}`]) {
      if (glyphs[key]) return key;
    }
    const shorter = name.replace(/[-_][^-_]+$/, "");
    if (shorter === name) break;
    name = shorter;
  }
  fallbackFiles.add(id);
  return "md-file_outline";
}

const stems = { ...normalize(source.file_stems), ...normalize(bridge.file_stems) };
const suffixes = { ...normalize(source.file_suffixes), ...normalize(bridge.file_suffixes) };
const languages = bridge.language_icons;
const fileIds = new Set([
  ...Object.keys(source.file_icons), ...Object.values(stems), ...Object.values(suffixes),
  ...Object.values(languages), ...glyphOverrides.keys(),
]);
for (const id of fileIds) fileIcons.set(id, fileGlyph(id));

function folderPair(pair) {
  const collapsed = basename(pair.collapsed, ".svg");
  const expanded = basename(pair.expanded, ".svg");
  const category = collapsed.replace(/^folder-/, "");
  let glyph = folderOverrides.get(category);
  if (!glyph) {
    const direct = `md-${collapsed.replaceAll("-", "_")}`;
    if (glyphs[direct]) glyph = direct;
  }
  if (!glyph) {
    fallbackFolders.add(category);
    glyph = "md-folder";
  }
  const openGlyph = glyph === "md-folder" ? "md-folder_open"
    : glyphs[`${glyph}_outline`] ? `${glyph}_outline` : glyph;
  fileIcons.set(collapsed, glyph);
  fileIcons.set(expanded, openGlyph);
  return { collapsed, expanded };
}
const directories = Object.fromEntries(Object.entries(normalize(source.named_directory_icons))
  .map(([name, pair]) => [asciiLower(name), folderPair(pair)]));
const genericDirectories = folderPair(source.directory_icons);
Object.assign(directories, normalize(bridge.named_directory_icons));

for (const id of [
  ...Object.values(stems), ...Object.values(suffixes), ...Object.values(languages),
  ...Object.values(genericDirectories), ...Object.values(directories).flatMap(Object.values),
]) {
  if (!fileIcons.has(id)) throw new Error(`Association references an undefined icon ID: ${id}`);
}

const quote = JSON.stringify;
const sorted = (entries) => Object.entries(entries).sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0);
const lines = [
  "# Generated by bun contrib/icon-theme/generate.mjs; edit material-nf.json instead.",
  `# Zed Material Icon Theme: ${materialRevision}`,
  `# Nerd Fonts: v3.4.0; glyphnames.json SHA-256: ${glyphDigest}`,
  "# Upstream name associations are retained; ASCII case variants are deduplicated.",
  "# SVG icon IDs are adapted to NF Material glyphs. See icons.LICENSE and contrib/icon-theme/README.md.",
  `# Associations: ${Object.keys(stems).length} file names, ${Object.keys(suffixes).length} suffixes, ${Object.keys(directories).length} directories.`,
  `# Generic NF fallbacks: ${fallbackFiles.size} file icon IDs, ${fallbackFolders.size} directory categories.`,
  "", "[file_icons]",
];
for (const [id, glyph] of sorted(Object.fromEntries(fileIcons))) {
  const code = glyphs[glyph].code.toUpperCase().padStart(8, "0");
  const fallback = fallbackFiles.has(id) ? "; generic fallback" : "";
  lines.push(`${quote(id)} = "\\U${code}" # nf-${glyph}${fallback}`);
}
for (const [table, entries] of [
  ["file_stems", stems], ["file_suffixes", suffixes], ["language_icons", languages],
  ["directory_icons", genericDirectories],
]) {
  lines.push("", `[${table}]`);
  for (const [name, id] of sorted(entries)) lines.push(`${quote(name)} = ${quote(id)}`);
}
lines.push("", "[named_directory_icons]");
for (const [name, pair] of sorted(directories)) {
  lines.push(`${quote(name)} = { collapsed = ${quote(pair.collapsed)}, expanded = ${quote(pair.expanded)} }`);
}
lines.push("");
const outputs = new Map([
  ["runtime/icons.toml", lines.join("\n")],
  ["runtime/icons.LICENSE", [
    "Association data adapted from zed-extensions/material-icon-theme (Apache-2.0):", zedLicense,
    "Material Icon Theme upstream data (MIT):", materialLicense,
    "Nerd Fonts glyph names and code points (licensing notice):", nfLicense,
  ].join("\n\n")],
]);
for (const [name, contents] of outputs) {
  const path = new URL(name, root);
  if (checkOnly) {
    if (await readFile(path, "utf8") !== contents) throw new Error(`${name} is out of date`);
  } else {
    await writeFile(path, contents);
  }
}
console.log(`${checkOnly ? "Verified" : "Generated"}: ${Object.keys(stems).length} file names, ${Object.keys(suffixes).length} suffixes, ${Object.keys(directories).length} directories, ${fileIcons.size} glyph definitions.`);
console.log(`Generic fallbacks: ${fallbackFiles.size} file IDs, ${fallbackFolders.size} directory categories.`);
