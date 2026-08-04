{
  description = "Utilities for working with oh-my-pi sessions and RPC";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";

    flake-parts.url = "github:hercules-ci/flake-parts";

    crane.url = "github:ipetkov/crane";

    advisory-db = {
      url = "github:rustsec/advisory-db";
      flake = false;
    };
  };

  outputs = inputs @ {flake-parts, ...}:
    flake-parts.lib.mkFlake {inherit inputs;} {
      systems = [
        "aarch64-darwin"
        "aarch64-linux"
        "x86_64-darwin"
        "x86_64-linux"
      ];

      perSystem = {pkgs, ...}: let
        inherit (pkgs) lib;

        craneLib = inputs.crane.mkLib pkgs;
        src = craneLib.cleanCargoSource ./.;
        workspace = builtins.fromTOML (builtins.readFile ./Cargo.toml);
        hasWorkspaceMembers = workspace.workspace.members != [];

        commonArgs = {
          inherit src;
          strictDeps = true;
          buildInputs = lib.optionals pkgs.stdenv.isDarwin [pkgs.libiconv];
        };

        cargoArtifacts = craneLib.buildDepsOnly commonArgs;
        cargoChecks = lib.optionalAttrs hasWorkspaceMembers {
          cargo-audit = craneLib.cargoAudit {
            inherit src;
            inherit (inputs) advisory-db;
          };

          cargo-clippy = craneLib.cargoClippy (commonArgs
            // {
              inherit cargoArtifacts;
              cargoClippyExtraArgs = "--workspace --all-targets --all-features -- --deny warnings";
            });

          cargo-deny = craneLib.cargoDeny {inherit src;};

          cargo-doc = craneLib.cargoDoc (commonArgs
            // {
              inherit cargoArtifacts;
              cargoDocExtraArgs = "--workspace --all-features";
              env.RUSTDOCFLAGS = "--deny warnings";
            });

          cargo-fmt = craneLib.cargoFmt {inherit src;};

          cargo-nextest = craneLib.cargoNextest (commonArgs
            // {
              inherit cargoArtifacts;
              cargoExtraArgs = "--workspace --all-features";
              cargoNextestPartitionsExtraArgs = "--no-tests=pass";
              partitionType = "count";
              partitions = 1;
            });
        };
      in {
        checks =
          {
            nix-fmt = pkgs.runCommand "omp-remote-nix-fmt" {nativeBuildInputs = [pkgs.alejandra];} ''
              alejandra --check ${./flake.nix}
              touch $out
            '';

            toml-fmt = craneLib.taploFmt {
              src = pkgs.lib.sources.sourceFilesBySuffices ./. [".toml"];
              taploExtraArgs = "--config ./taplo.toml";
            };

            wasm-protocol = craneLib.cargoTest (commonArgs
              // {
                inherit cargoArtifacts;
                cargoExtraArgs = "-p omp-control-protocol --target wasm32-unknown-unknown --test wasm";
                nativeBuildInputs = [pkgs.lld pkgs.nodejs pkgs.wasm-bindgen-cli];
              });
          }
          // cargoChecks;

        devShells.default = craneLib.devShell {
          packages = with pkgs; [
            alejandra
            cargo-audit
            cargo-deny
            cargo-nextest
            lld
            nodejs
            rust-analyzer
            taplo
            wasm-bindgen-cli
          ];
        };

        formatter = pkgs.alejandra;
      };
    };
}
