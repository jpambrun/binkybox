## Build Command

Use `cargo build -r` for builds in this repository.

## Task Runner

Use `just` targets for local workflow:

- `just kill`: kill any running `binkybox` process.
- `just build`: build with `cargo build -r`.
- `just run`: run `target\\release\\binkybox.exe` (depends on `kill`).
- `just buildrun`: build then run, replacing any currently running version.

When code changes are complete, run `just buildrun`.
