{
  inputs = {
    rust-overlay.url = "github:oxalica/rust-overlay";
    nixpkgs.url = "github:NixOS/nixpkgs";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
      flake-utils,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        overlays = [ (import rust-overlay) ];

        pkgs = import nixpkgs { inherit system overlays; };

        rust = pkgs.rust-bin.fromRustupToolchainFile ./rust/rust-toolchain.toml;

        rustPlatform = pkgs.makeRustPlatform {
          cargo = rust;
          rustc = rust;
        };

        mina_txn_hasher = pkgs.callPackage ./ops/mina/mina_txn_hasher.nix { };

        frameworks = pkgs.darwin.apple_sdk.frameworks;

        buildDependencies =
          with pkgs;
          [ rustPlatform.bindgenHook ]
          ++ lib.optionals stdenv.isDarwin [
            frameworks.Security
            frameworks.CoreServices
            lld_20 # A faster linker
          ]
          ++ lib.optionals (!stdenv.isDarwin) [
            mold-wrapped # Linux only - https://github.com/rui314/mold#mold-a-modern-linker
          ];

        # used to ensure rustfmt is nightly version to support unstable features
        nightlyToolchain = pkgs.rust-bin.selectLatestNightlyWith (
          toolchain: toolchain.minimal.override { extensions = [ "rustfmt" ]; }
        );

        developmentDependencies =
          with pkgs;
          [
            biome
            cargo-audit
            cargo-machete
            cargo-nextest
            clang # For clang in shell
            curl
            check-jsonschema
            git # Needed but not declared by Nix's 'stdenv' build.
            hurl
            jq
            nightlyToolchain.passthru.availableComponents.rustfmt
            nix-output-monitor # Use 'nom' in place of 'nix' to use this.
            nixfmt-rfc-style # For formatting Nix code.
            openssh # Needed by 'git' but not declared.
            rclone
            ruby
            rubyPackages.standard
            rubyPackages.rspec
            rust
            shellcheck
            shfmt
            mdformat
            samply # rust profiling
          ]
          ++ buildDependencies;

        cargo-toml = builtins.fromTOML (builtins.readFile ./rust/Cargo.toml);
      in
      with pkgs;
      {
        packages = flake-utils.lib.flattenTree rec {
          inherit mina_txn_hasher;

          mina-indexer = rustPlatform.buildRustPackage rec {
            meta = with lib; {
              homepage = "https://github.com/Granola-Team/mina-indexer";
              license = licenses.asl20;
              mainProgram = "mina-indexer";
              platforms = platforms.all;
              maintainers = [ ];
            };

            pname = cargo-toml.package.name;
            version = cargo-toml.package.version;

            src = lib.cleanSourceWith {
              src = lib.cleanSource ./.;
              filter =
                path: type:
                (path != ".direnv")
                && ((path == "rust/.cargo") || (path == "rust/.cargo/config.toml") || (dirOf path != "rust/.cargo"))
                && (path != "result")
                && (path != ".build")
                && (path != "rust/target")
                && (path != "ops")
                && (path != "Justfile")
                && (path != "Rakefile")
                && (path != "tests");
            };

            cargoLock = {
              lockFile = ./rust/Cargo.lock;
              # Needed until a fix for https://github.com/async-graphql/async-graphql/issues/1703 is published
              outputHashes = {
                "async-graphql-7.0.16" = "sha256-O/r89nSwwDL7u06NgQhzjgKyrEuMS4euULPT5SmUA4E=";
              };
            };

            nativeBuildInputs = buildDependencies;

            # This is equivalent to `git rev-parse --short=8 HEAD`
            gitCommitHash = builtins.substring 0 8 (self.rev or (abort "Nix build requires a clean Git repo."));

            postPatch = ''ln -s "${./rust/Cargo.lock}" Cargo.lock'';
            preBuild = ''
              export GIT_COMMIT_HASH=${gitCommitHash}
              cd rust
            '';
            doCheck = false;
            preInstall = "mkdir -p $out/var/lib/mina-indexer";
          };

          default = mina-indexer;

          # In-image block puller for the --fetch-new-blocks-exe /
          # --missing-block-recovery-exe hooks (self-contained: bundles its
          # curl/grep/sed/coreutils deps, so the minimal image needs nothing).
          mesa-pull = pkgs.writeShellApplication {
            name = "mesa-pull";
            runtimeInputs = with pkgs; [ curl gnugrep gnused coreutils ];
            # the script tolerates per-block failures, so no `errexit`
            bashOptions = [ "nounset" "pipefail" ];
            text = builtins.readFile ./ops/mesa-mut/mesa-pull.sh;
          };

          # Production OCI image. Uses streamLayeredImage for cache-friendly
          # layers, ships only the runtime closure (no source tree), runs as a
          # non-root user, and is reproducible (pinned `created`).
          dockerImage = pkgs.dockerTools.streamLayeredImage {
            name = "mina-indexer";
            tag = builtins.substring 0 8 (self.rev or "dev");
            created = "1970-01-01T00:00:01Z";
            contents = with pkgs; [
              mina-indexer
              mesa-pull # /bin/mesa-pull for the fetch/recovery hooks
              bash
              cacert # CA roots for any outbound HTTPS (block/ledger fetch)
              dockerTools.fakeNss # /etc/passwd & /etc/group so the non-root user resolves
            ];
            # World-writable data + tmp dirs. /tmp must exist: the indexer writes
            # temp snapshot files there during ingestion (the minimal image has
            # no /tmp otherwise, so startup fails with ENOENT).
            fakeRootCommands = ''
              mkdir -p ./data ./tmp
              chmod 1777 ./data ./tmp
            '';
            config = {
              # Cmd (not Entrypoint) so callers can override, e.g.
              #   docker run IMAGE mina-indexer server start --help
              Cmd = [ "${pkgs.lib.getExe mina-indexer}" ];
              Env = [ "TMPDIR=/tmp" ];
              User = "65534:65534"; # nobody — never root
              WorkingDir = "/data";
              Volumes = { "/data" = { }; };
              ExposedPorts = { "8080/tcp" = { }; };
            };
          };
        };

        devShells.default = mkShell {
          buildInputs = developmentDependencies ++ lib.optional (!stdenv.isDarwin) mina_txn_hasher; # for backwards compatibility
        };
      }
    );
}
