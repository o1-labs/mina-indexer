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

        # Baked genesis ledgers (state dumps) for the configless network images.
        # Fetched at build time (fixed-output), decompressed to /data on first boot.
        mesaGenesisGz = pkgs.fetchurl {
          url = "https://storage.googleapis.com/o1labs-gitops-infrastructure/mina-mesa-mut-1/mina-mesa-mut-1-state-dump-3NLp6dKNhYtsqUj49QYV5GtDaeocSJBAa2y2ER2QQLqLukE3wuZT-df71a5f2dd5abdae8e2e5d4d0047b383bdfca4d75ec1d2260b8ad621f1a18ffe.json.gz";
          sha256 = "004li2l5c319730czsav8bqqpysb2fs48sri6vqxav3m9zvf68qr";
        };
        # devnet genesis ledger = the o1labs-gitops state dump matching the embedded
        # checkpoint block (devnet-527922-3NK4DL35, ledger hash jwX3YJh…).
        devnetGenesisGz = pkgs.fetchurl {
          url = "https://storage.googleapis.com/o1labs-gitops-infrastructure/devnet/devnet-state-dump-3NK4DL35iKQ6G8VPqPFLZ122M82dcRRPt8rHrpRW662kXWpH8fRa-8ad77d2bc0adcd51d5837fabe00e6a76c69f00f547e112cdb0534b425d2d9c3b.json.gz";
          sha256 = "0a7kld30iwmly23bx2x625l6haihlrfv227wxh0qvf9j65mbxxw6";
        };
        # Factory for a configless per-network indexer image. Everything the
        # indexer needs is baked in; the per-network entrypoint runs it with no
        # args. Mirrors the generic `dockerImage` hardening (non-root, /data
        # volume, TLS env, /tmp). `genesisGz`, when set, is placed at
        # /genesis/<net>.json.gz for the entrypoint to decompress on first boot.
        mkIndexerImage =
          {
            net,
            indexer,
            verifyBlock,
            fetcher,
            entry,
            genesisGz ? null,
          }:
          let
            genesisContents = pkgs.lib.optional (genesisGz != null) (
              pkgs.runCommand "mina-indexer-genesis-${net}" { } ''
                mkdir -p $out/genesis
                cp ${genesisGz} $out/genesis/${net}.json.gz
              ''
            );
          in
          pkgs.dockerTools.streamLayeredImage {
            name = "mina-indexer";
            tag = "${builtins.substring 0 8 (self.rev or "dev")}-${net}";
            created = "1970-01-01T00:00:01Z";
            contents =
              [
                indexer
                fetcher
                verifyBlock
                entry
              ]
              ++ (with pkgs; [
                bash
                gzip
                cacert
                dockerTools.fakeNss
              ])
              ++ genesisContents;
            fakeRootCommands = ''
              mkdir -p ./data ./tmp
              chmod 1777 ./data ./tmp
            '';
            config = {
              Cmd = [ "${pkgs.lib.getExe entry}" ];
              Env = [
                "TMPDIR=/tmp"
                "SSL_CERT_FILE=${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt"
                "CURL_CA_BUNDLE=${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt"
              ];
              User = "65534:65534";
              WorkingDir = "/data";
              Volumes = {
                "/data" = { };
              };
              ExposedPorts = {
                "8080/tcp" = { };
              };
            };
          };
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

          # In-image block-verification shim for the --verify-block-exe hook:
          # forwards the block to the mina-verify-server sidecar (trustless mode).
          verify-block = pkgs.writeShellApplication {
            name = "verify-block";
            runtimeInputs = with pkgs; [ curl ];
            bashOptions = [ "nounset" "pipefail" ];
            text = builtins.readFile ./ops/mesa-mut/verify-block.sh;
          };

          # Generic block puller for the PUBLIC networks (mainnet, devnet): pulls
          # from gs://mina_network_block_data, whose objects are already named
          # <network>-<height>-<hash>.json. Mesa keeps its own mesa-pull (different
          # bucket + multi-dash prefix rewrite).
          block-pull = pkgs.writeShellApplication {
            name = "block-pull";
            runtimeInputs = with pkgs; [ curl gnugrep gnused coreutils ];
            bashOptions = [ "nounset" "pipefail" ];
            text = builtins.readFile ./ops/block-pull.sh;
          };

          # Configless per-network entrypoints: each exec's mina-indexer with all
          # flags baked in (zero args/mounts needed). Each per-network file holds
          # only its variables (NETWORK / GENESIS_HASH / FETCH_EXE / GENESIS_GZ);
          # the shared body (checkpoints, genesis decompression, retention, the
          # server-start invocation) lives once in common.sh and is concatenated
          # at build time. mesa/devnet decompress their baked genesis on first boot.
          mkEntry =
            {
              net,
              needsGzip ? false,
            }:
            pkgs.writeShellApplication {
              name = "indexer-entry-${net}";
              runtimeInputs = [ mina-indexer ] ++ pkgs.lib.optional needsGzip pkgs.gzip;
              bashOptions = [ "errexit" "nounset" "pipefail" ];
              text =
                builtins.readFile (./ops/entrypoints + "/${net}.sh")
                + builtins.readFile ./ops/entrypoints/common.sh;
            };
          indexer-entry-mainnet = mkEntry { net = "mainnet"; };
          indexer-entry-devnet = mkEntry {
            net = "devnet";
            needsGzip = true;
          };
          indexer-entry-mesa = mkEntry {
            net = "mesa";
            needsGzip = true;
          };

          # Per-network configless images (one repo, tag suffix -<network>):
          #   ghcr.io/o1-labs/mina-indexer:<tag>-mainnet | -devnet | -mesa-mut
          "dockerImage-mainnet" = mkIndexerImage {
            net = "mainnet";
            indexer = mina-indexer;
            verifyBlock = verify-block;
            fetcher = block-pull;
            entry = indexer-entry-mainnet;
          };
          "dockerImage-devnet" = mkIndexerImage {
            net = "devnet";
            indexer = mina-indexer;
            verifyBlock = verify-block;
            fetcher = block-pull;
            entry = indexer-entry-devnet;
            genesisGz = devnetGenesisGz;
          };
          "dockerImage-mesa" = mkIndexerImage {
            net = "mesa";
            indexer = mina-indexer;
            verifyBlock = verify-block;
            fetcher = mesa-pull;
            entry = indexer-entry-mesa;
            genesisGz = mesaGenesisGz;
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
              verify-block # /bin/verify-block for the --verify-block-exe hook
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
              # SSL_CERT_FILE: the bundled curl (in mesa-pull) has no built-in CA
              # path, so without this every HTTPS block/ledger fetch fails TLS
              # verification ("curl failed to verify the legitimacy of the
              # server") and the fetcher silently pulls 0 blocks.
              Env = [
                "TMPDIR=/tmp"
                "SSL_CERT_FILE=${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt"
                "CURL_CA_BUNDLE=${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt"
              ];
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
