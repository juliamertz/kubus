{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixpkgs-unstable";
    crane.url = "github:ipetkov/crane";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = {
    nixpkgs,
    crane,
    rust-overlay,
    ...
  }: let
    systems = ["aarch64-darwin" "x86_64-darwin" "x86_64-linux" "aarch64-linux"];
    overlays = [(import rust-overlay)];
    craneLibFor = pkgs: (crane.mkLib pkgs).overrideToolchain (p: p.rust-bin.stable.latest.default);

    eachSystem = nixpkgs.lib.genAttrs systems;
  in {
    devShells = eachSystem (system: let
      pkgs = import nixpkgs {inherit system overlays;};
      craneLib = craneLibFor pkgs;
    in {
      default = craneLib.devShell {
        packages = let
          toolchain = pkgs.rust-bin.stable.latest.default.override {
            extensions = ["rust-src" "rustfmt"];
          };
        in
          with pkgs;
          with toolchain; [
            cargo-expand 
            rust-analyzer
            clippy
          ];
      };
    });
  };
}
