{
  description = "Local-first coding project planner backed by ACP agents";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    crane.url = "github:ipetkov/crane";
    flake-utils.url = "github:numtide/flake-utils";
    treefmt-nix.url = "github:numtide/treefmt-nix";
    git-hooks.url = "github:cachix/git-hooks.nix";
  };

  outputs = {
    self,
    nixpkgs,
    rust-overlay,
    crane,
    flake-utils,
    treefmt-nix,
    git-hooks,
    ...
  }:
    flake-utils.lib.eachDefaultSystem (system: let
      pkgs = import nixpkgs {
        inherit system;
        overlays = [(import rust-overlay)];
      };

      rustToolchain = pkgs.rust-bin.stable.latest.default.override {
        extensions = ["rustfmt" "clippy"];
        targets = ["wasm32-unknown-unknown"];
      };
      craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;
      guiAssetSource = path: let
        rel = pkgs.lib.removePrefix "${toString ./.}/" (toString path);
      in
        pkgs.lib.hasPrefix "crates/tzu-gui/style" rel
        || pkgs.lib.hasPrefix "crates/tzu-gui/public/static" rel;
      src = pkgs.lib.cleanSourceWith {
        src = ./.;
        filter = path: type:
          craneLib.filterCargoSources path type || guiAssetSource path;
      };
      nativeBuildInputs = with pkgs; [
        git
        pkg-config
      ];
      buildInputs = with pkgs; [
        openssl
        postgresql
        sqlite
      ];
      commonArgs = {
        inherit src nativeBuildInputs buildInputs;
        strictDeps = true;
      };
      cargoArtifacts = craneLib.buildDepsOnly commonArgs;
      package = craneLib.buildPackage (commonArgs // {inherit cargoArtifacts;});
      treefmtEval = treefmt-nix.lib.evalModule pkgs (import ./nix/treefmt.nix);
      pre-commit-check = git-hooks.lib.${system}.run {
        src = ./.;
        hooks = import ./nix/pre-commit.nix {
          inherit pkgs;
          treefmtWrapper = treefmtEval.config.build.wrapper;
          inherit rustToolchain;
        };
      };
    in {
      packages.default = package;
      formatter = treefmtEval.config.build.wrapper;
      checks = {
        default = package;
        formatting = treefmtEval.config.build.check self;
        clippy = craneLib.cargoClippy (commonArgs
          // {
            inherit cargoArtifacts;
            cargoClippyExtraArgs = "--all-targets --all-features -- --deny warnings";
          });
        fmt = craneLib.cargoFmt {inherit src;};
      };
      devShells.default = craneLib.devShell {
        checks = self.checks.${system};
        packages = with pkgs;
          [
            cargo-leptos
            cargo-nextest
            nodejs
            playwright-driver
            pre-commit
            rust-analyzer
            wasm-bindgen-cli
          ]
          ++ nativeBuildInputs
          ++ buildInputs
          ++ pre-commit-check.enabledPackages;
        LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath buildInputs;
        shellHook = pre-commit-check.shellHook;
      };
    });
}
