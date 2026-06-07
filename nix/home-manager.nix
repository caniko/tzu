self: {
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.programs.tzu;
  package = cfg.package;
  tomlString = value: builtins.toJSON (toString value);
  tomlStringList = values: builtins.toJSON (map toString values);
  tomlBool = value:
    if value
    then "true"
    else "false";
  projectRoots =
    lib.optional (cfg.projectsDirectory != null) cfg.projectsDirectory
    ++ cfg.projectsDirectories;
  primaryProjectRoot =
    if cfg.projectsDirectory != null
    then cfg.projectsDirectory
    else builtins.head cfg.projectsDirectories;
  configToml = lib.concatStringsSep "\n" (
    lib.optional (cfg.projectsDirectory != null)
    "projects_directory = ${tomlString cfg.projectsDirectory}"
    ++ lib.optional (cfg.projectsDirectories != [])
    "projects_directories = ${tomlStringList cfg.projectsDirectories}"
    ++ [
      "include_nested_contexts = ${tomlBool cfg.includeNestedContexts}"
      ""
      "[gui]"
      "host = ${tomlString cfg.gui.host}"
      "port = ${toString cfg.gui.port}"
      "enable_service = ${tomlBool cfg.gui.enable}"
      ""
    ]
  );
in {
  options.programs.tzu = {
    enable = lib.mkEnableOption "tzu local-first planning harness";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.system}.default;
      defaultText = lib.literalExpression "self.packages.\${pkgs.system}.default";
      description = "Package providing the tzu and tzu-gui binaries.";
    };

    projectsDirectory = lib.mkOption {
      type = lib.types.nullOr lib.types.path;
      default = null;
      description = "Legacy single directory whose direct child projects are discovered by tzu.";
    };

    projectsDirectories = lib.mkOption {
      type = lib.types.listOf lib.types.path;
      default = [];
      description = "Directories whose direct child projects are discovered by tzu.";
    };

    includeNestedContexts = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = "Whether context traversal and GUI @ folder discovery should descend into nested repositories and submodules by default.";
    };

    databaseUrl = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      description = "Optional TZU_DATABASE_URL value for the tzu-gui user service.";
    };

    gui = {
      enable = lib.mkEnableOption "the tzu GUI user service";

      host = lib.mkOption {
        type = lib.types.str;
        default = "127.0.0.1";
        description = "Host address for the tzu-gui user service.";
      };

      port = lib.mkOption {
        type = lib.types.port;
        default = 7070;
        description = "Port for the tzu-gui user service.";
      };
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = !cfg.gui.enable || projectRoots != [];
        message = "programs.tzu.projectsDirectory or programs.tzu.projectsDirectories must be set when programs.tzu.gui.enable is true.";
      }
    ];

    home.packages = [package];

    xdg.configFile."tzu/config.toml".text = configToml;

    systemd.user.services.tzu-gui = lib.mkIf cfg.gui.enable {
      Unit = {
        Description = "tzu GUI";
        After = ["network.target"];
      };
      Service =
        {
          ExecStart = "${lib.getExe' package "tzu-gui"} --project-root ${lib.escapeShellArg (toString primaryProjectRoot)} --host ${lib.escapeShellArg cfg.gui.host} --port ${toString cfg.gui.port}";
          Restart = "on-failure";
        }
        // lib.optionalAttrs (cfg.databaseUrl != null) {
          Environment = "TZU_DATABASE_URL=${cfg.databaseUrl}";
        };
      Install.WantedBy = ["default.target"];
    };
  };
}
