{
  description = "Sparksmith — development shell with the pinned Rust toolchain for `cargo xtask ci`";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    # The exact toolchain in rust-toolchain.toml, including clippy, rustfmt and the
    # wasm32 target that `cargo xtask ci` checks. nixpkgs' rustc has no wasm target.
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    { nixpkgs, rust-overlay, ... }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
      ];
      forAllSystems =
        f:
        nixpkgs.lib.genAttrs systems (
          system:
          f (
            import nixpkgs {
              inherit system;
              overlays = [ rust-overlay.overlays.default ];
            }
          )
        );

      # Linux only (adapted from Solidcraft's flake): winit dlopens Wayland/X11, wgpu
      # loads EGL/Vulkan and rfd reaches the desktop portal through libdbus. macOS
      # uses AppKit and Metal and needs none of these.
      runtimeLibs =
        pkgs:
        pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux (
          with pkgs;
          [
            wayland
            libxkbcommon
            libglvnd
            vulkan-loader
            dbus
            libx11
            libxcursor
            libxi
            libxrandr
            libxrender
            libxext
            libxcb
          ]
        );
    in
    {
      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          name = "sparksmith";
          packages = [
            (pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml)
            pkgs.git
            pkgs.sccache
          ]
          ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [ pkgs.pkg-config ];
          LD_LIBRARY_PATH = pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isLinux (
            "${pkgs.lib.makeLibraryPath (runtimeLibs pkgs)}:/run/opengl-driver/lib"
          );
          shellHook = ''
            # A separate target directory keeps this toolchain's cache apart from rustup's,
            # unless the caller already chose one (for example a target shared by worktrees).
            export CARGO_TARGET_DIR="''${CARGO_TARGET_DIR:-target/nix}"
            # sccache shares compiled crates across worktrees and target directories.
            # Set RUSTC_WRAPPER to an empty string to build without it.
            export RUSTC_WRAPPER="''${RUSTC_WRAPPER-sccache}"
            export SCCACHE_CACHE_SIZE="''${SCCACHE_CACHE_SIZE:-60G}"
            # Paths under these directories are made relative before hashing, so the same
            # crate compiled in another worktree or target directory is a cache hit. The
            # sccache server reads them when it starts: after adding a worktree, run
            # `sccache --stop-server` once so the next build picks the new list up.
            if [ -z "''${SCCACHE_BASEDIRS:-}" ]; then
              dirs=$(git worktree list --porcelain 2>/dev/null | sed -n 's/^worktree //p' | tr '\n' ':')
              case "$CARGO_TARGET_DIR" in /*) dirs="$dirs$CARGO_TARGET_DIR:" ;; esac
              export SCCACHE_BASEDIRS="''${dirs%:}"
            fi
            echo "sparksmith: $(rustc --version); run cargo xtask ci"
          '';
        };
      });

      formatter = forAllSystems (pkgs: pkgs.nixfmt);
    };
}
