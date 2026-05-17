# Release Process

Pila releases are GitHub-release-driven. Publishing a release in the GitHub UI
triggers [`.github/workflows/release.yml`](../.github/workflows/release.yml),
which:

- builds and pushes a multi-arch image for `linux/amd64` and `linux/arm64`
- publishes `ghcr.io/johanneskast/pila:<tag>` and `ghcr.io/johanneskast/pila:latest`
- generates an SPDX JSON SBOM artifact (`pila-sbom.spdx.json`)
- signs the published image digest with keyless cosign via GitHub OIDC

## Prepare the release commit

1. Update `version` in `Cargo.toml`.
2. Move the relevant notes from `## [Unreleased]` in `CHANGELOG.md` into a new
   versioned section.
3. Update the compare links at the bottom of `CHANGELOG.md`.
4. Run the usual checks locally:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
```

## Publish from GitHub

Example for `v0.1.0`:

1. Push the release commit to `master`.
2. Open `GitHub -> Releases -> Draft a new release`.
3. Choose or create the tag `v0.1.0`.
4. Fill out the release notes and click `Publish release`.

Publishing the release starts the workflow. No manual Docker publish step is
needed. The same workflow can also be started manually from the Actions tab via
`workflow_dispatch`, but the normal path is publishing a GitHub release.

## Watch the workflow with `gh`

```bash
gh run list --workflow release.yml --limit 1
gh run watch <run-id> --exit-status
```

When the run succeeds, these install targets should exist:

- `ghcr.io/johanneskast/pila:v0.1.0`
- `ghcr.io/johanneskast/pila:latest`

The workflow summary also records the pushed image digest, and the Actions run
contains the uploaded SBOM artifact.
