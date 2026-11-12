{
  lib,
  rustPlatform,
  callPackage,
  runCommand,
  installShellFiles,
  git,
  formats,
  gitRev ? null,
  grammarOverlays ? [ ],
  includeGrammarIf ? _: true,
}:
let
  fs = lib.fileset;
  readTOML = f: builtins.fromTOML (builtins.readFile f);
  cargoTOML = readTOML ../helix-term/Cargo.toml;
  wsTOML = readTOML ../Cargo.toml;

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
      ../languages.toml
      ../nix
      (fs.fileFilter (file: lib.strings.hasInfix ".git" file.name) ../.)
      (fs.fileFilter (file: file.hasExt "svg") ../.)
      (fs.fileFilter (file: file.hasExt "md") ../.)
      (fs.fileFilter (file: file.hasExt "nix") ../.)
    ]
  );

  # Keep only grammars that are not already provided by Nix in the build config.
  languagesAll = readTOML ../languages.toml;
  languagesMin = (formats.toml { }).generate "helix-languages.toml" (
    (builtins.removeAttrs languagesAll [ "grammar" ])
    // {
      grammar = builtins.filter (g: !builtins.elem g.name grammars.names) languagesAll.grammar;
    }
  );

  # Build grammars separately and link them into the runtime directory.
  grammars = callPackage ./grammars.nix { inherit grammarOverlays includeGrammarIf; };
  runtimeDir = runCommand "helix-runtime" { } ''
    mkdir -p $out
    ln -s ${../runtime}/* $out
    rm -f $out/grammars
    ln -s ${grammars} $out/grammars
  '';

  # The unwrapped editor can be built without references to the grammars.
  helix-unwrapped = rustPlatform.buildRustPackage (self: {
    pname = "helix-unwrapped";
    version = wsTOML.workspace.package.version;
    src = fs.toSource {
      root = ../.;
      fileset = src;
    };

    cargoLock = {
      lockFile = ../Cargo.lock;
      # Git dependencies may be added temporarily without specifying outputHashes.
      allowBuiltinFetchGit = true;
    };

    postPatch = ''
      cp ${languagesMin} languages.toml
    '';

    nativeBuildInputs = [
      installShellFiles
      git
    ];
    buildType = "release";

    # Grammars are built by Nix rather than Helix's network-enabled build script.
    HELIX_DISABLE_AUTO_GRAMMAR_BUILD = "1";
    HELIX_NIX_BUILD_REV = gitRev;
    doCheck = false;
    strictDeps = true;

    postInstall = ''
      mkdir -p $out/lib
      installShellCompletion ${../contrib/completion}/hx.{bash,fish,zsh}
      mkdir -p $out/share/{applications,icons/hicolor/{256x256,scalable}/apps}
      cp ${../contrib/Helix.desktop} $out/share/applications/Helix.desktop
      cp ${../logo.svg} $out/share/icons/hicolor/scalable/apps/helix.svg
      cp ${../contrib/helix.png} $out/share/icons/hicolor/256x256/apps/helix.png
    '';

    meta = {
      inherit (wsTOML.workspace.package) homepage;
      inherit (cargoTOML.package) description;
      mainProgram = cargoTOML.package.default-run;
      license = lib.licenses.mpl20;
    };
  });
in
# Helix discovers this runtime next to its executable without affecting child processes.
runCommand "helix-${helix-unwrapped.version}"
  {
    inherit (helix-unwrapped) version meta;
    passthru = {
      unwrapped = helix-unwrapped;
      inherit runtimeDir grammars;
    };
  }
  ''
    cp -r ${helix-unwrapped} $out
    chmod -R u+w $out
    mkdir -p $out/libexec/helix
    ln -s ${runtimeDir} $out/libexec/helix/runtime
    for f in $out/bin/*; do
      mv "$f" $out/libexec/helix/
      ln -s "../libexec/helix/$(basename "$f")" "$f"
    done
  ''
