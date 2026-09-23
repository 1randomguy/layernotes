{
  description = "Bottom-layer Wayland post-it notes stored as markdown files";

  inputs = {
    crane.url = "github:ipetkov/crane";
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs = {
        nixpkgs.follows = "nixpkgs";
      };
    };
  };

  outputs =
    {
      crane,
      nixpkgs,
      rust-overlay,
      ...
    }:
    let
      forAllSystems = with nixpkgs; (lib.genAttrs lib.systems.flakeExposed);
      perSystem = forAllSystems (system: rec {
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };
        craneLib = crane.mkLib pkgs;
        rustToolchain = pkgs.rust-bin.stable.latest;
        buildInputs = with pkgs; [
          rustToolchain.default
          rustPlatform.bindgenHook
          pkg-config
          libxkbcommon
          libGL
          wayland
          vulkan-loader
        ];
        runtimeDependencies = with pkgs; [
          wayland
          mesa
          vulkan-loader
          libGL
          libglvnd
        ];
        ldLibraryPath = pkgs.lib.makeLibraryPath runtimeDependencies;

        # Filter source to only include files needed by the Rust build.
        src = pkgs.lib.cleanSourceWith {
          src = ./.;
          filter = craneLib.filterCargoSources;
        };

        commonArgs = {
          inherit src;
          strictDeps = true;
          nativeBuildInputs = with pkgs; [
            makeWrapper
            pkg-config
            autoPatchelfHook
          ];
          inherit buildInputs runtimeDependencies ldLibraryPath;
        };

        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        defaultPackage = craneLib.buildPackage (
          commonArgs
          // {
            inherit cargoArtifacts;
            postInstall = ''
              wrapProgram "$out/bin/layernotes" --prefix LD_LIBRARY_PATH : "${ldLibraryPath}"
            '';
            meta.mainProgram = "layernotes";
          }
        );

        devShell = pkgs.mkShell {
          inherit ldLibraryPath;
          buildInputs = buildInputs ++ [
            pkgs.rust-analyzer-unwrapped
            pkgs.nixfmt
          ];

          RUST_SRC_PATH = "${rustToolchain.rust-src}/lib/rustlib/src/rust/library";
          LD_LIBRARY_PATH = ldLibraryPath;
        };

        formatter = pkgs.nixfmt;
      });
    in
    {
      packages = forAllSystems (system: {
        default = perSystem.${system}.defaultPackage;
      });
      devShells = forAllSystems (system: {
        default = perSystem.${system}.devShell;
      });
      formatter = forAllSystems (system: perSystem.${system}.formatter);
    };
}
