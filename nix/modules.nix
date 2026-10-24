{self}: {
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.services.harbor-cad;
in {
  options.services.harbor-cad = {
    enable = lib.mkEnableOption "harbor-cad user worker package (activation remains operator-owned)";
    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
    };
  };
  config = lib.mkIf cfg.enable {environment.systemPackages = [cfg.package];};
}
