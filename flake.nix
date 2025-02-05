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
          buildInputs = [
            pkgs.openssl
          ];
          nativeBuildInputs = [
            pkgs.pkg-config
            (pkgs.rust-bin.nightly."2025-02-01".minimal.override {
              targets = [
                "riscv64gc-unknown-linux-gnu"
                "aarch64-apple-darwin"
                "aarch64-unknown-linux-gnu"
              ];
              extensions = [
                "rust-src"
                "clippy"
              ];
            })
          ];
        };
    };
}
