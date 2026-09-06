{
  description = "notifyd — agent-first notification service. One Rust binary, Postgres only.";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    let
      version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;
      commit = self.shortRev or self.dirtyShortRev or "unknown";
    in
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        notifyd = pkgs.rustPlatform.buildRustPackage {
          pname = "notifyd";
          inherit version;
          src = pkgs.lib.cleanSource ./.;
          cargoLock.lockFile = ./Cargo.lock;

          nativeBuildInputs = [ pkgs.pkg-config ];
          buildInputs = [ pkgs.openssl pkgs.curl ];

          # build.rs reads it for /v1/health; there is no .git inside the sandbox.
          GIT_COMMIT_SHA = commit;

          # Unit tests only; nothing here needs a database.
          doCheck = true;

          meta = with pkgs.lib; {
            description = "Agent-first self-hosted notification service (email, SMS, WhatsApp, push, in-app) with an MCP server";
            homepage = "https://github.com/rmzlb/notifyd";
            license = licenses.mit;
            mainProgram = "notifyd";
            platforms = platforms.linux ++ platforms.darwin;
          };
        };
      in
      {
        packages.default = notifyd;
        packages.notifyd = notifyd;

        apps.default = flake-utils.lib.mkApp { drv = notifyd; };

        devShells.default = pkgs.mkShell {
          packages = with pkgs; [ cargo rustc rustfmt clippy rust-analyzer pkg-config openssl curl postgresql_16 ];
          RUST_LOG = "notifyd=debug";
        };
      })
    // {
      # NixOS: services.notifyd.enable = true; secrets in an EnvironmentFile.
      nixosModules.default = { config, lib, pkgs, ... }:
        let cfg = config.services.notifyd;
        in {
          options.services.notifyd = {
            enable = lib.mkEnableOption "notifyd notification service";
            package = lib.mkOption {
              type = lib.types.package;
              default = self.packages.${pkgs.system}.default;
              description = "notifyd package to run.";
            };
            environmentFile = lib.mkOption {
              type = lib.types.path;
              description = "File with DATABASE_URL, JWT_SECRET, ADMIN_API_KEY and provider credentials (see docker-compose.yml for the full list).";
            };
            port = lib.mkOption { type = lib.types.port; default = 3400; };
            environment = lib.mkOption {
              type = lib.types.attrsOf lib.types.str;
              default = { };
              description = "Non-secret environment (EMAIL_PROVIDER, EMAIL_FROM, PUBLIC_URL, EMAIL_RATE_PER_SEC…).";
            };
          };
          config = lib.mkIf cfg.enable {
            systemd.services.notifyd = {
              description = "notifyd notification service";
              wantedBy = [ "multi-user.target" ];
              after = [ "network-online.target" "postgresql.service" ];
              wants = [ "network-online.target" ];
              environment = cfg.environment // { PORT = toString cfg.port; };
              serviceConfig = {
                ExecStart = lib.getExe cfg.package;   # migrations are embedded (sqlx::migrate!)
                EnvironmentFile = cfg.environmentFile;
                DynamicUser = true;
                Restart = "always";
                RestartSec = 2;
                NoNewPrivileges = true;
                ProtectSystem = "strict";
                ProtectHome = true;
                PrivateTmp = true;
              };
            };
          };
        };
    };
}
