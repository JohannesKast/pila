// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Johannes Kast
//
// Regression guard: the release binary reads some resources from disk at
// runtime (relative to its WORKDIR). The multi-stage Dockerfile builds the
// binary in one stage and copies only selected paths into the slim runtime
// image — so any such directory that is NOT explicitly COPY'd is missing at
// runtime and the app panics on first access (e.g. "Cannot read translation
// file locales/de.ftl").
//
// These tests fail at `cargo test` time — long before a broken image ships —
// if a runtime resource directory is not wired into the Dockerfile.

use std::fs;
use std::path::Path;

/// Directories the release binary reads from disk **at runtime**.
///
/// How to know whether something belongs here:
///   - `locales/` → `translations::load_all` does `std::fs::read_to_string` on
///     `locales/{code}.ftl` for each supported locale.
///   - `static/` → served by `ServeDir::new("static")` in `build_router`.
///
/// What does NOT belong here (embedded into the binary at compile time, so it
/// needs no COPY): Askama templates (`#[template(path = …)]`), SQL migrations
/// (`sqlx::migrate!("./migrations")`), and the jersey presets (a `const` JSON
/// string in `jersey.rs`).
const RUNTIME_RESOURCE_DIRS: &[&str] = &["locales", "static"];

/// Lines of the final (runtime) build stage of the Dockerfile — everything
/// after the last `FROM`. Resource `COPY`s must live here, not in the builder.
fn runtime_stage_lines() -> Vec<String> {
    let dockerfile = fs::read_to_string("Dockerfile").expect("Dockerfile must exist at crate root");
    let lines: Vec<&str> = dockerfile.lines().collect();
    let last_from = lines
        .iter()
        .rposition(|l| l.trim_start().to_ascii_uppercase().starts_with("FROM "))
        .expect("Dockerfile must contain a FROM instruction");
    lines[last_from..].iter().map(|l| l.to_string()).collect()
}

#[test]
fn runtime_resource_dirs_exist_in_repo() {
    // Keeps RUNTIME_RESOURCE_DIRS honest: a typo or a deleted directory here
    // would otherwise let the Dockerfile assertion pass against a phantom.
    for dir in RUNTIME_RESOURCE_DIRS {
        assert!(
            Path::new(dir).is_dir(),
            "Declared runtime resource '{dir}/' does not exist in the repo. \
             Update RUNTIME_RESOURCE_DIRS if it was intentionally removed."
        );
    }
}

#[test]
fn dockerfile_copies_every_runtime_resource_dir() {
    let runtime_stage = runtime_stage_lines();

    for dir in RUNTIME_RESOURCE_DIRS {
        // Match a COPY whose destination is the resource dir under WORKDIR,
        // e.g. `COPY --from=builder ... /app/locales ./locales`. We accept any
        // destination token that ends in the dir name to stay tolerant of
        // `./static`, `static`, or `/app/static` spellings.
        let copied = runtime_stage.iter().any(|line| {
            let l = line.trim_start();
            if !l.to_ascii_uppercase().starts_with("COPY ") {
                return false;
            }
            l.split_whitespace().any(|tok| {
                let tok = tok.trim_end_matches('/');
                tok == *dir || tok.ends_with(&format!("/{dir}"))
            })
        });

        assert!(
            copied,
            "Dockerfile runtime stage never COPYs '{dir}/' into the image. \
             The binary reads it at runtime, so the container will panic on \
             first access. Add: \
             `COPY --from=builder --chown=appuser:appuser /app/{dir} ./{dir}`"
        );
    }
}

#[test]
fn dockerignore_does_not_exclude_runtime_resources() {
    // `COPY . .` in the builder stage respects .dockerignore; if a resource
    // dir is ignored it never reaches the builder and the runtime COPY copies
    // nothing. Catch an over-broad ignore pattern that names a resource dir.
    let Ok(ignore) = fs::read_to_string(".dockerignore") else {
        return; // no .dockerignore → nothing excluded
    };
    for dir in RUNTIME_RESOURCE_DIRS {
        for raw in ignore.lines() {
            let pat = raw.trim();
            if pat.is_empty() || pat.starts_with('#') {
                continue;
            }
            let normalized = pat.trim_start_matches('/').trim_end_matches('/');
            assert!(
                normalized != *dir,
                ".dockerignore excludes runtime resource '{dir}/' (pattern '{raw}'), \
                 so it never reaches the build context."
            );
        }
    }
}

#[test]
fn base_template_uses_built_css_not_cdn() {
    // The styling must come from the build-time bundle, not the Tailwind Play
    // CDN (dev-only, third-party dependency, unstyled page if unreachable).
    let base = fs::read_to_string("templates/base.html").expect("base.html must exist");
    assert!(
        base.contains("/static/app.css"),
        "base.html must link the compiled /static/app.css"
    );
    assert!(
        !base.contains("cdn.tailwindcss.com"),
        "base.html must not load the Tailwind Play CDN — it is not for production. \
         Use the build-time bundle (static/app.css)."
    );
    let css = Path::new("static/app.css");
    assert!(
        css.is_file() && fs::metadata(css).map(|m| m.len() > 0).unwrap_or(false),
        "static/app.css must be built and non-empty (run `npm run build:css`)"
    );
}

#[test]
fn servedir_paths_are_declared_runtime_resources() {
    // Auto-detect drift: if someone adds another `ServeDir::new("…")` the path
    // becomes a new runtime dependency. Force it to be registered in
    // RUNTIME_RESOURCE_DIRS (and thus checked against the Dockerfile above).
    let main_rs = fs::read_to_string("src/main.rs").expect("src/main.rs must exist");
    let needle = "ServeDir::new(\"";
    let mut search = main_rs.as_str();
    while let Some(idx) = search.find(needle) {
        let rest = &search[idx + needle.len()..];
        let path = &rest[..rest.find('"').expect("unterminated ServeDir path literal")];
        // Only a top-level dir name maps cleanly to a Dockerfile COPY target.
        let top = path
            .trim_start_matches("./")
            .split('/')
            .next()
            .unwrap_or(path);
        assert!(
            RUNTIME_RESOURCE_DIRS.contains(&top),
            "ServeDir serves '{path}' but '{top}' is not in RUNTIME_RESOURCE_DIRS. \
             Add it there and COPY it in the Dockerfile, or the release image \
             will 404 / fail to serve those files."
        );
        search = rest;
    }
}
