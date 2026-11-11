{
  description = "multisig_module — RFP-005 prototype Logos core module: create and execute a private M-of-N multisig on LEZ v0.4";

  nixConfig = {
    extra-substituters = [ "https://cache.nix.logos.co/public" ];
    extra-trusted-public-keys = [ "public:l4HrXgL4nw246+LBh2SOJyhz64BoGegOYLheT/iIAPU=" ];
  };

  inputs = {
    logos-module-builder = {
      url = "github:logos-co/logos-module-builder";
      # The builder's pinned rust-overlay predates rustc 1.98.1 (needed by the LEZ v0.4 crates).
      inputs.rust-overlay.follows = "rust-overlay";
    };
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs = inputs@{ self, logos-module-builder, ... }:
    let
      nixpkgs = logos-module-builder.inputs.nixpkgs;
      # aarch64-linux only (what Docker on Apple Silicon builds): the rapidsnark libs below are the Logos fork's -fPIC linux archives
      systems = [ "aarch64-linux" ]; # x86_64-linux: add its rapidsnark zip hash below
      rapidsnarkVersion = "0.0.8";
      rapidsnarkHash = { aarch64-linux = "sha256-X2Wa+CmRgTxX52HyrGsjM1WRyG7pEcL1hBdyx4rwmZ8="; };
      rapidsnarkSlug = { aarch64-linux = "aarch64"; };

      # The builder copies rust-lib/ alone, so hand it a tree with real copies of what rust-lib/Cargo.toml
      # reaches outside it: ../../../vendor (-> rust-lib/vendor), crates/msig-core and msig-host
      # (symlinks locally; msig-methods is the in-tree prebuilt-guest stand-in). Needs a git+file flake
      # ref so ../../crates resolves (see modules/nix-docker.sh).
      srcFor = system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          slug = "rapidsnark-linux-${rapidsnarkSlug.${system}}-pic-v${rapidsnarkVersion}";
          rapidsnarkZip = pkgs.fetchurl {
            url = "https://github.com/logos-blockchain/logos-blockchain-rust-rapidsnark/releases/download/rapidsnark-pic-v${rapidsnarkVersion}/${slug}.zip";
            hash = rapidsnarkHash.${system};
          };
        in
        pkgs.runCommand "multisig-module-src" { nativeBuildInputs = [ pkgs.unzip ]; } ''
          cp -r ${./.} $out
          chmod -R u+w $out
          cd $out/rust-lib
          rm -rf crates/msig-core crates/msig-host ../logos-rust-sdk-src
          cp -r ${../../crates/msig-core} crates/msig-core
          cp -r ${../../crates/msig-host} crates/msig-host
          cp -r ${../../vendor} vendor
          mkdir -p third_party/rapidsnark/prebuilt
          unzip -q ${rapidsnarkZip} -d $TMPDIR/rs
          cp $TMPDIR/rs/${slug}/lib/* third_party/rapidsnark/prebuilt/
          chmod -R u+w .
          sed -i 's#path = "../../../vendor/#path = "vendor/#; s#^\[workspace\]$#[workspace]\nexclude = ["vendor"]#' Cargo.toml
        '';

      moduleFor = system: logos-module-builder.lib.mkLogosModule {
        src = srcFor system;
        configFile = ./metadata.json;
        flakeInputs = inputs;
      };
    in
    {
      packages = nixpkgs.lib.genAttrs systems (system: (moduleFor system).packages.${system});
    };
}
