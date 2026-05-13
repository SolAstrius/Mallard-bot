{
  description = "Mallard bot — dev shell (Rust toolchain stays under rustup)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };

        # Libraries rustup-installed toolchains and built binaries dynamically
        # link against on NixOS. Add to LD_LIBRARY_PATH so cargo-built artefacts
        # find their loader without nix-ld.
        runtimeLibs = with pkgs; [
          stdenv.cc.cc.lib
          zlib
        ];
      in {
        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            # Toolchain manager — Rust itself is NOT pinned by the flake.
            # Use `rustup default stable` (or any channel you want).
            rustup

            # Runtime: video pipeline shells out to ffmpeg.
            ffmpeg-headless

            # Build-time helpers for crates that probe the environment.
            pkg-config
            cmake
          ];

          buildInputs = runtimeLibs;

          env = {
            LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath runtimeLibs;
            # Keep cargo registries / target dir on a fast local path.
            CARGO_TARGET_DIR = "target";
          };

          shellHook = ''
            export PATH="$HOME/.cargo/bin:$PATH"
            if ! rustup show active-toolchain >/dev/null 2>&1; then
              echo "[mallard-bot] no rustup toolchain active — run 'rustup default stable'"
            fi
          '';
        };
      });
}
