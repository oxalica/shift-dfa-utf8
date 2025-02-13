{
  inputs = {
    rust-overlay.url = "github:oxalica/rust-overlay";
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs =
    { nixpkgs, rust-overlay, ... }:
    {
      devShells.x86_64-linux.default =
        let
          pkgs = import nixpkgs {
            system = "x86_64-linux";
            overlays = [
              rust-overlay.overlays.default
            ];
          };
        in
        pkgs.mkShell {
          nativeBuildInputs = [
            (pkgs.rust-bin.nightly."2025-02-01".default.override {
              targets = [
                "riscv64gc-unknown-linux-gnu"
                "aarch64-apple-darwin"
                "i686-unknown-linux-gnu"
                "arm-unknown-linux-gnueabi"
              ];
              extensions = [
                "llvm-tools-preview"
              ];
            })
            pkgs.cargo-llvm-cov
          ];
        };

      devShells.x86_64-linux.i686 =
        let
          pkgs = import nixpkgs {
            system = "x86_64-linux";
            crossSystem = "i686-linux";
            overlays = [
              rust-overlay.overlays.default
            ];
          };
        in
        pkgs.mkShell {
          nativeBuildInputs = [
            (pkgs.buildPackages.rust-bin.nightly."2025-02-01".minimal.override {
              targets = [
                "i686-unknown-linux-gnu"
              ];
            })
          ];

          env.CARGO_TARGET_I686_UNKNOWN_LINUX_GNU_LINKER = "i686-unknown-linux-gnu-cc";
        };
    };
}
