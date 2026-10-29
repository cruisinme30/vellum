// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Embeds the built web app (`web/dist`, from `pnpm --dir web build`) in
//! the binary.
//!
//! Writes `$OUT_DIR/web_app.rs`: a table of every file under the folder,
//! each with its bytes through `include_bytes!`. When the folder has no
//! `index.html` (the web app was not built, as in the Rust-only CI job),
//! the table holds a one-page placeholder saying so, so `cargo build` never
//! needs pnpm. `VELLUM_WEB_DIST` points at another folder.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::{env, fs, io};

const PLACEHOLDER: &str = "<!doctype html>\n<html lang=\"en\">\n<head>\n\
<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
<title>Vellum</title>\n</head>\n<body>\n<main>\n<h1>The web app is not built</h1>\n\
<p>This <code>vellum</code> binary was compiled without the web app. Build it with \
<code>pnpm --dir web build</code>, then build the binary again. The API is available \
under <code>/api</code>.</p>\n</main>\n</body>\n</html>\n";

/// Every file under `dir`, as (`/relative/path`, absolute path). Source
/// maps are left out: they are most of the build's size, and the binary
/// has a 40 MB budget.
fn files(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            files(root, &path, out)?;
        } else if path.extension().is_none_or(|extension| extension != "map")
            && let Ok(relative) = path.strip_prefix(root)
        {
            let url: Vec<String> = relative
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect();
            out.push((format!("/{}", url.join("/")), path.clone()));
        }
    }
    Ok(())
}

fn main() -> io::Result<()> {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap_or_default());
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap_or_default());
    let dist = env::var_os("VELLUM_WEB_DIST").map_or_else(
        || manifest.join("..").join("..").join("web").join("dist"),
        PathBuf::from,
    );
    let mut table = Vec::new();
    let built = dist.join("index.html").is_file();
    if built {
        files(&dist, &dist, &mut table)?;
        table.sort();
    } else {
        let placeholder = out_dir.join("web-app-placeholder.html");
        fs::write(&placeholder, PLACEHOLDER)?;
        table.push(("/index.html".to_owned(), placeholder));
    }
    let mut code = String::new();
    let _ = writeln!(
        code,
        "/// Whether the real web app was embedded (not the placeholder).\n\
         pub const BUILT: bool = {built};\n\n\
         /// Every file of the web app: its URL path and bytes.\n\
         pub static FILES: &[(&str, &[u8])] = &["
    );
    for (url, path) in &table {
        let _ = writeln!(code, "    ({url:?}, include_bytes!({:?})),", path.display());
    }
    code.push_str("];\n");
    fs::write(out_dir.join("web_app.rs"), code)?;
    // Cargo re-runs a script on every build while a watched path is
    // missing, so without a build the parent folder is watched instead:
    // creating `dist` changes it, and the next build embeds the app.
    let watched = match dist.parent() {
        Some(parent) if !dist.exists() && parent.is_dir() => parent,
        _ => &dist,
    };
    println!("cargo::rerun-if-changed={}", watched.display());
    println!("cargo::rerun-if-env-changed=VELLUM_WEB_DIST");
    Ok(())
}
