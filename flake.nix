{
  description = "Local-first coding project planner backed by ACP agents";

  inputs = {
    rs-harbor.url = "git+https://codeberg.org/caniko/rs-harbor.git?ref=trunk&rev=9bfa8bdb0ecb22d7bc11448665f7fbaebae7a759";
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    crane.url = "github:ipetkov/crane";
    flake-utils.url = "github:numtide/flake-utils";
    treefmt-nix = {
      url = "github:numtide/treefmt-nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    git-hooks = {
      url = "github:cachix/git-hooks.nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    plinth = {
      url = "git+https://codeberg.org/caniko/plinth.git?ref=refs/heads/trunk";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.flake-utils.follows = "flake-utils";
    };
  };

  outputs = {
    self,
    rs-harbor,
    nixpkgs,
    rust-overlay,
    crane,
    flake-utils,
    treefmt-nix,
    git-hooks,
    plinth,
    ...
  }: let
    perSystem = flake-utils.lib.eachDefaultSystem (system: let
      pkgs = import nixpkgs {
        inherit system;
        overlays = [(import rust-overlay)];
      };

      rustToolchain = pkgs.rust-bin.stable.latest.default.override {
        extensions = ["rustfmt" "clippy"];
        targets = ["wasm32-unknown-unknown"];
      };
      craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;
      buildCache = rs-harbor.lib.mkBuildCachePolicy {
        inherit pkgs;
        sccachePackage = rs-harbor.packages.${system}.sccache;
        cacheRoot = null;
        namespaceScope = "canix-rust";
        namespaceGeneration = 5;
      };
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
      tzu-dev-config = pkgs.writeShellApplication {
        name = "tzu-dev-config";
        runtimeInputs = with pkgs; [git gnused];
        text = ''
          root="$(git -C "''${PWD}" rev-parse --show-toplevel 2>/dev/null || pwd -P)"
          exec "$root/scripts/tzu-dev-config" "$@"
        '';
      };
      cargoArtifacts = craneLib.buildDepsOnly commonArgs;
      package = buildCache.withRustCache {
        package = craneLib.buildPackage (commonArgs // {inherit cargoArtifacts;});
      };
      website = plinth.lib.${system}.mkProjectSite {
        pname = "tzu-website";
        domain = "tzu.tartanoglu.com";
        configPath = ./website/plinth-project.toml;
      };
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
      packages = {
        default = package;
        website = website;
        site = website;
      };
      apps.deploy-pages = plinth.lib.${system}.mkDeployPagesApp {
        domain = "tzu.tartanoglu.com";
      };
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
        home-manager-module = pkgs.callPackage ./nix/home-manager-module-test.nix {
          module = import ./nix/home-manager.nix self;
        };
      };
      devShells.default = craneLib.devShell {
        checks = self.checks.${system};
        packages = with pkgs;
          [
            cargo-leptos
            cargo-nextest
            nodejs
            python3
            playwright-driver
            pre-commit
            rust-analyzer
            tzu-dev-config
            wasm-bindgen-cli
          ]
          ++ nativeBuildInputs
          ++ buildInputs
          ++ pre-commit-check.enabledPackages;
        LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath buildInputs;
        shellHook = ''
          ${pre-commit-check.shellHook}
        '';
      };
    });
  in
    perSystem
    // {
      homeModules.default = self.homeModules.tzu;
      homeModules.tzu = import ./nix/home-manager.nix self;
    };
}
