{
  harbor-meta,
  harbor-rs,
  harbor-py,
  rustfmtPackage,
}: _: {
  imports = [
    harbor-meta.treefmtModules.nix
    harbor-meta.treefmtModules.toml
    harbor-meta.treefmtModules.markdown
    harbor-meta.treefmtModules.shell
    harbor-meta.treefmtModules.svg
    harbor-rs.treefmtModules.rust
    harbor-py.treefmtModules.python
  ];
  projectRootFile = "flake.nix";
  settings.excludes = [
    "templates/**"
    "**/templates/**"
    "**/tests/fixtures/**"
    "**/checks/fixtures/**"
    # Simit owns workflow serialization.
    ".github/workflows/*.yaml"
    # Eval prompts, graders and scaffolds are test inputs.
    ".opencode/skills/cad-robustness-loop/evals/**"
    # Prettier strips the significant spaces from the literal ` ;; ` separator.
    "docs/materials-and-cost.md"
  ];
  programs.rustfmt = {
    package = rustfmtPackage;
    edition = "2024";
  };
}
