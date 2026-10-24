{self}: {
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.programs.harbor-cad;
in {
  options.programs.harbor-cad = {
    enable = lib.mkEnableOption "harbor-cad CLI and MCP";
    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
    };
    mcp.enable = lib.mkOption {
      type = lib.types.bool;
      default = true;
    };
    mcp.package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.mcp;
    };
    worker.enable = lib.mkEnableOption "opt-in persistent local worker";
    worker.profile = lib.mkOption {
      type = lib.types.path;
      description = "Explicit HostExecutionProfile; no automatic GPU route or host changes";
    };
  };
  config = lib.mkIf cfg.enable {
    home.packages = [cfg.package] ++ lib.optional cfg.mcp.enable cfg.mcp.package;
    systemd.user.services.harbor-cad = lib.mkIf cfg.worker.enable {
      Unit.Description = "Local harbor-cad worker";
      Service = {
        ExecStart = "${cfg.package}/bin/harbor-cad worker --state %h/.local/state/harbor-cad --profile ${cfg.worker.profile}";
        Restart = "on-failure";
        UMask = "0077";
        NoNewPrivileges = true;
        RestrictAddressFamilies = "AF_UNIX";
        # Worker must create job cgroups through its user manager; native payloads are separately isolated.
        MemoryDenyWriteExecute = true;
      };
      Install.WantedBy = ["default.target"];
    };
  };
}
