{
  harbor-meta,
  harbor-rs,
  harbor-py,
  rustfmtPackage,
}: _: {
  imports = [
    harbor-meta.treefmtModules.nix
    harbor-meta.treefmtModules.toml
    harbor-rs.treefmtModules.rust
    harbor-py.treefmtModules.python
  ];
  projectRootFile = "flake.nix";
  # Templates and byte-sensitive fixtures keep their source policy;
  # simit owns workflow serialization.
  settings.excludes = ["**/templates/**" "**/tests/fixtures/**" "**/checks/fixtures/**" ".github/workflows/*.yaml"];
  programs.rustfmt = {
    package = rustfmtPackage;
    edition = "2024";
  };
}
