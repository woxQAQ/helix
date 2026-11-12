{
  lib,
  rustPlatform,
  callPackage,
  runCommand,
  installShellFiles,
  git,
  gitRev ? null,
  grammarOverlays ? [ ],
  includeGrammarIf ? _: true,
}:
let
  fs = lib.fileset;

  src = fs.difference (fs.gitTracked ../.) (
    fs.unions [
      ../.envrc
      ../rustfmt.toml
      ../screenshot.png
      ../book
      ../docs
      # The built-in icon theme is embedded in the binary at compile time.
      (fs.difference ../runtime ../runtime/icons.toml)
      ../flake.lock
      ../nix
      (fs.fileFilter (file: lib.strings.hasInfix ".git" file.name) ../.)
      (fs.fileFilter (file: file.hasExt "svg") ../.)
      (fs.fileFilter (file: file.hasExt "md") ../.)
      (fs.fileFilter (file: file.hasExt "nix") ../.)
    ]
  );

  # Build grammars separately and link them into the runtime directory.
  grammars = callPackage ./grammars.nix { inherit grammarOverlays includeGrammarIf; };
  runtimeDir = runCommand "helix-runtime" { } ''
    mkdir -p $out
    ln -s ${../runtime}/* $out
    rm -r $out/grammars
    ln -s ${grammars} $out/grammars
  '';
in
rustPlatform.buildRustPackage (self: {
  cargoLock = {
    lockFile = ../Cargo.lock;
    # Git dependencies may be added temporarily without specifying outputHashes.
    allowBuiltinFetchGit = true;
  };

  propagatedBuildInputs = [ runtimeDir ];
  nativeBuildInputs = [
    installShellFiles
    git
  ];
  buildType = "release";
  name = with builtins; (fromTOML (readFile ../helix-term/Cargo.toml)).package.name;
  src = fs.toSource {
    root = ../.;
    fileset = src;
  };

  # Grammars are built by Nix rather than Helix's network-enabled build script.
  HELIX_DISABLE_AUTO_GRAMMAR_BUILD = "1";
  HELIX_NIX_BUILD_REV = gitRev;
  doCheck = false;
  strictDeps = true;
  env.HELIX_DEFAULT_RUNTIME = "${runtimeDir}";

  postInstall = ''
    mkdir -p $out/lib
    installShellCompletion ${../contrib/completion}/hx.{bash,fish,zsh}
    mkdir -p $out/share/{applications,icons/hicolor/{256x256,scalable}/apps}
    cp ${../contrib/Helix.desktop} $out/share/applications/Helix.desktop
    cp ${../logo.svg} $out/share/icons/hicolor/scalable/apps/helix.svg
    cp ${../contrib/helix.png} $out/share/icons/hicolor/256x256/apps/helix.png
  '';

  meta.mainProgram = "hx";
})
