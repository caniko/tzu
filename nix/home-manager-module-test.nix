{
  lib,
  pkgs,
  module,
}: let
  eval = lib.evalModules {
    modules = [
      {
        options = {
          assertions = lib.mkOption {
            type = lib.types.listOf lib.types.attrs;
            default = [];
          };
          home = {
            username = lib.mkOption {type = lib.types.str;};
            homeDirectory = lib.mkOption {type = lib.types.path;};
            stateVersion = lib.mkOption {type = lib.types.str;};
            packages = lib.mkOption {
              type = lib.types.listOf lib.types.package;
              default = [];
            };
          };
          xdg = {
            enable = lib.mkOption {
              type = lib.types.bool;
              default = false;
            };
            configFile = lib.mkOption {
              type = lib.types.attrsOf (lib.types.submodule {
                options.text = lib.mkOption {type = lib.types.lines;};
              });
              default = {};
            };
          };
          systemd.user.services = lib.mkOption {
            type = lib.types.attrsOf lib.types.attrs;
            default = {};
          };
        };
      }
      {
        home = {
          username = "tester";
          homeDirectory = "/home/tester";
          stateVersion = "25.05";
        };
        xdg.enable = true;
        programs.tzu = {
          enable = true;
          package = pkgs.writeShellScriptBin "tzu-gui" ''
            echo tzu-gui "$@"
          '';
          projectsDirectory = /home/tester/Projects;
          includeNestedContexts = true;
          databaseUrl = "sqlite:///home/tester/.local/state/tzu/state.sqlite";
          gui = {
            enable = true;
            host = "127.0.0.1";
            port = 9090;
          };
        };
      }
      module
    ];
    specialArgs = {};
  };
  configText = eval.config.xdg.configFile."tzu/config.toml".text;
  service = eval.config.systemd.user.services.tzu-gui.Service;
in
  pkgs.runCommand "tzu-home-manager-module-test" {} ''
    grep -q 'projects_directory = "/home/tester/Projects"' ${pkgs.writeText "config.toml" configText}
    grep -q 'include_nested_contexts = true' ${pkgs.writeText "config2.toml" configText}
    test "${service.Environment}" = "TZU_DATABASE_URL=sqlite:///home/tester/.local/state/tzu/state.sqlite"
    touch "$out"
  ''
