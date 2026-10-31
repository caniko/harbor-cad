# Changelog

## Unreleased

- Accept Python 3.14 (`requires-python = ">=3.12,<3.15"`); the suite passes on 3.12 and 3.14.

## 0.1.0 - 2026-10-26

- First tagged release, distributed under GPL-3.0-or-later (see `LICENSE`).
- Accept `mcp>=2.2.0,<3` and `jsonschema>=4.26.0,<5` instead of exact pins so
  distribution packages can supply compatible versions; `uv.lock` still pins
  the tested set. The build backend accepts `hatchling>=1.29.0`.
- Move the `harbor-cad-mcp` package manifest and `uv.lock` into `python/` so this
  package owns its release directory. Earlier history is in the git history of
  the parent `components/cad` directory, which has no changelog.
