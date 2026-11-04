{
  description = "Fast, safe MTP photo backup and culling";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs = { self, nixpkgs, rust-overlay, ... }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
      pkgsFor = system: import nixpkgs {
        inherit system;
        overlays = [ rust-overlay.overlays.default ];
      };
    in {
      packages = forAllSystems (system:
        let
          pkgs = pkgsFor system;
          rust = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
          rustPlatform = pkgs.makeRustPlatform {
            cargo = rust;
            rustc = rust;
          };
          nativeBuildInputs = [ pkgs.pkg-config pkgs.cmake pkgs.nasm ];
          buildInputs = [
            pkgs.udev
            pkgs.libxkbcommon
            pkgs.wayland
            pkgs.libGL
            pkgs.libx11
            pkgs.libxcursor
            pkgs.libxrandr
            pkgs.libxi
          ];
        in {
          default = rustPlatform.buildRustPackage {
            pname = "mtp-cull";
            version = "0.1.0";
            src = ./.;
            cargoLock.lockFile = ./Cargo.lock;
            cargoBuildFeatures = [ "turbo" ];
            cargoTestFeatures = [ "turbo" ];
            inherit nativeBuildInputs buildInputs;
          };
        });
      checks = forAllSystems (system: {
        default = self.packages.${system}.default;
      });
      devShells = forAllSystems (system:
        let
          pkgs = pkgsFor system;
          rust = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
        in {
          default = pkgs.mkShell {
            packages = [
              rust
              pkgs.gcc
              pkgs.cmake
              pkgs.nasm
              pkgs.pkg-config
              pkgs.udev
              pkgs.libxkbcommon
              pkgs.wayland
              pkgs.libGL
              pkgs.libx11
              pkgs.libxcursor
              pkgs.libxrandr
              pkgs.libxi
            ];
          };
        });
    };
}
