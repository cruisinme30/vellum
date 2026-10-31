// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Install channel manifests for a release (ADR 0022, D250).
//!
//! `cargo xtask installers --version <x.y.z> --sums <SHA256SUMS> --out <dir>
//! [--repo <owner/name>]` reads the release's checksums and writes:
//!
//! - `homebrew/vellum.rb`: the Homebrew formula for the tap, which installs
//!   the macOS universal binary or the static Linux binary;
//! - `winget/manifests/v/Vellum/Vellum/<version>/`: the three winget
//!   manifests for the portable Windows binaries.
//!
//! Every file points at the GitHub release download URLs, so the channels
//! ship exactly the binaries that the release signed.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// Release files each channel needs, by name.
pub const MACOS: &str = "vellum-macos-universal";
/// The static Linux binary for x86-64.
pub const LINUX_AMD64: &str = "vellum-linux-amd64";
/// The static Linux binary for 64-bit ARM, including Raspberry Pi 3 and later.
pub const LINUX_ARM64: &str = "vellum-linux-arm64";
/// The portable Windows binary for x86-64.
pub const WINDOWS_X64: &str = "vellum-windows-x64.exe";
/// The portable Windows binary for ARM64.
pub const WINDOWS_ARM64: &str = "vellum-windows-arm64.exe";

/// The winget package identifier.
const WINGET_ID: &str = "Vellum.Vellum";
/// The winget manifest schema version.
const WINGET_SCHEMA: &str = "1.9.0";
const DESCRIPTION: &str =
    "Self-hosted workspace where every object is a Markdown file";
const HOMEPAGE: &str = "https://cruisinme30.github.io/vellum/";

struct Options {
    version: String,
    sums: PathBuf,
    out: PathBuf,
    repo: String,
}

fn parse(args: &[String]) -> Result<Options, String> {
    let (mut version, mut sums, mut out) = (None, None, None);
    let mut repo = "cruisinme30/vellum".to_owned();
    let mut iter = args.iter();
    while let Some(flag) = iter.next() {
        let value =
            iter.next().ok_or_else(|| format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--version" => version = Some(value.clone()),
            "--sums" => sums = Some(PathBuf::from(value)),
            "--out" => out = Some(PathBuf::from(value)),
            "--repo" => repo.clone_from(value),
            other => return Err(format!("unknown flag {other}")),
        }
    }
    let version = version.ok_or("--version is required")?;
    check_version(&version)?;
    if repo.split('/').count() != 2 || repo.chars().any(char::is_whitespace) {
        return Err(format!("--repo must be owner/name, not {repo}"));
    }
    Ok(Options {
        version,
        sums: sums.ok_or("--sums is required")?,
        out: out.ok_or("--out is required")?,
        repo,
    })
}

/// A release version is `x.y.z` with an optional `-pre` suffix, and nothing
/// that could break out of a quoted string in the generated files.
fn check_version(version: &str) -> Result<(), String> {
    let core = version.split_once('-').map_or(version, |(core, _)| core);
    let numeric = core.split('.').count() == 3
        && core.split('.').all(|part| {
            !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit())
        });
    let safe = version
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-');
    if numeric && safe {
        Ok(())
    } else {
        Err(format!("--version must look like 1.2.3, not {version}"))
    }
}

/// Writes the manifests.
pub fn run(args: &[String]) -> Result<(), String> {
    let options = parse(args)?;
    let text = std::fs::read_to_string(&options.sums)
        .map_err(|e| format!("cannot read {}: {e}", options.sums.display()))?;
    let sums = parse_sums(&text)?;
    let release = Release {
        version: &options.version,
        repo: &options.repo,
        sums: &sums,
    };
    let mut files =
        vec![(PathBuf::from("homebrew/vellum.rb"), formula(&release)?)];
    files.extend(winget(&release)?);
    for (path, contents) in files {
        write(&options.out.join(&path), &contents)?;
        #[expect(
            clippy::print_stdout,
            reason = "xtask reports to the terminal"
        )]
        {
            println!("wrote {}", options.out.join(path).display());
        }
    }
    Ok(())
}

fn write(path: &Path, contents: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    std::fs::write(path, contents)
        .map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// Reads `sha256sum` output: a lowercase hex digest, two spaces (or a space
/// and `*`), then the file name.
pub fn parse_sums(text: &str) -> Result<BTreeMap<String, String>, String> {
    let mut sums = BTreeMap::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let (digest, name) = line
            .split_once(' ')
            .ok_or_else(|| format!("not a checksum line: {line}"))?;
        let name = name.trim_start_matches([' ', '*']);
        let valid =
            digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit());
        if !valid || name.is_empty() {
            return Err(format!("not a checksum line: {line}"));
        }
        let _ = sums.insert(name.to_owned(), digest.to_ascii_lowercase());
    }
    Ok(sums)
}

struct Release<'a> {
    version: &'a str,
    repo: &'a str,
    sums: &'a BTreeMap<String, String>,
}

impl Release<'_> {
    fn url(&self, file: &str) -> String {
        format!(
            "https://github.com/{}/releases/download/v{}/{file}",
            self.repo, self.version
        )
    }

    fn sha256(&self, file: &str) -> Result<&str, String> {
        self.sums
            .get(file)
            .map(String::as_str)
            .ok_or_else(|| format!("SHA256SUMS has no {file}"))
    }
}

/// The Homebrew formula: prebuilt binaries, a `brew services` definition
/// that runs `vellum serve`, and a version test.
fn formula(release: &Release<'_>) -> Result<String, String> {
    let mut text = String::new();
    let block = |file: &str, indent: &str| -> Result<String, String> {
        Ok(format!(
            "{indent}url \"{}\"\n{indent}sha256 \"{}\"\n",
            release.url(file),
            release.sha256(file)?
        ))
    };
    let _ = write!(
        text,
        r##"# typed: false
# frozen_string_literal: true

# Generated by `cargo xtask installers` for Vellum {version}. Do not edit:
# the release workflow replaces this file on every release.
class Vellum < Formula
  desc "{DESCRIPTION}"
  homepage "{HOMEPAGE}"
  version "{version}"
  license "AGPL-3.0-only"

  on_macos do
{macos}  end

  on_linux do
    on_intel do
{amd64}    end
    on_arm do
{arm64}    end
  end

  def install
    bin.install Dir["vellum-*"].first => "vellum"
  end

  def post_install
    (var/"vellum").mkpath
  end

  service do
    run [opt_bin/"vellum", "serve"]
    environment_variables VELLUM_DATA_DIR: var/"vellum"
    keep_alive true
    log_path var/"log/vellum.log"
    error_log_path var/"log/vellum.log"
  end

  test do
    assert_match version.to_s, shell_output("#{{bin}}/vellum --version")
  end
end
"##,
        version = release.version,
        macos = block(MACOS, "    ")?,
        amd64 = block(LINUX_AMD64, "      ")?,
        arm64 = block(LINUX_ARM64, "      ")?,
    );
    Ok(text)
}

/// The winget version, installer and default locale manifests.
fn winget(release: &Release<'_>) -> Result<Vec<(PathBuf, String)>, String> {
    let version = release.version;
    let dir =
        PathBuf::from(format!("winget/manifests/v/Vellum/Vellum/{version}"));
    let header = |kind: &str| {
        format!(
            "# yaml-language-server: \
             $schema=https://aka.ms/winget-manifest.{kind}.{WINGET_SCHEMA}.sche\
             ma.json\n# Generated by `cargo xtask installers`.\n\
             PackageIdentifier: {WINGET_ID}\nPackageVersion: {version}\n"
        )
    };
    let footer = |kind: &str| {
        format!("ManifestType: {kind}\nManifestVersion: {WINGET_SCHEMA}\n")
    };

    let mut versions = header("version");
    versions.push_str("DefaultLocale: en-US\n");
    versions.push_str(&footer("version"));

    let mut installer = header("installer");
    installer.push_str(
        "InstallerType: portable\nCommands:\n  - vellum\nInstallers:\n",
    );
    for (arch, file) in [("x64", WINDOWS_X64), ("arm64", WINDOWS_ARM64)] {
        let _ = write!(
            installer,
            "  - Architecture: {arch}\n    InstallerUrl: {}\n    \
                InstallerSha256: {}\n",
            release.url(file),
            release.sha256(file)?.to_ascii_uppercase()
        );
    }
    installer.push_str(&footer("installer"));

    let mut locale = header("defaultLocale");
    let _ = write!(
        locale,
        "PackageLocale: en-US\n\
         Publisher: The Vellum contributors\n\
         PublisherUrl: https://github.com/{repo}\n\
         PackageName: Vellum\n\
         PackageUrl: {HOMEPAGE}\n\
         License: AGPL-3.0-only\n\
         LicenseUrl: https://github.com/{repo}/blob/main/LICENSE\n\
         ShortDescription: {DESCRIPTION}\n\
         ReleaseNotesUrl: https://github.com/{repo}/releases/tag/v{version}\n\
         Tags:\n  - markdown\n  - notes\n  - wiki\n  - self-hosted\n",
        repo = release.repo,
    );
    locale.push_str(&footer("defaultLocale"));

    Ok(vec![
        (dir.join(format!("{WINGET_ID}.yaml")), versions),
        (dir.join(format!("{WINGET_ID}.installer.yaml")), installer),
        (dir.join(format!("{WINGET_ID}.locale.en-US.yaml")), locale),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(n: u8) -> String {
        format!("{n:x}").repeat(64)
    }

    fn sums() -> BTreeMap<String, String> {
        let mut text = String::new();
        for (name, n) in
            [MACOS, LINUX_AMD64, LINUX_ARM64, WINDOWS_X64, WINDOWS_ARM64]
                .iter()
                .zip(1_u8..)
        {
            let _ = writeln!(text, "{}  {name}", digest(n));
        }
        parse_sums(&text).unwrap()
    }

    #[test]
    fn reads_sha256sum_output() {
        let text = format!("{}  a\n{} *b.exe\n\n", digest(10), "A".repeat(64));
        let sums = parse_sums(&text).unwrap();
        assert_eq!(sums["a"], digest(10));
        assert_eq!(sums["b.exe"], "a".repeat(64));
        assert!(parse_sums("abc  file").is_err());
        assert!(parse_sums(&format!("{}  ", digest(1))).is_err());
        assert!(parse_sums("nospace").is_err());
    }

    #[test]
    fn versions_are_checked() {
        for good in ["1.0.0", "0.12.3", "1.0.0-rc.1"] {
            assert!(check_version(good).is_ok(), "{good}");
        }
        for bad in ["1.0", "v1.0.0", "1.0.0\"", "1.0.x", "1..0", "1.0.0 rc"] {
            assert!(check_version(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn flags_are_checked() {
        let args = |list: &[&str]| {
            list.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>()
        };
        assert!(parse(&args(&["--version", "1.0.0", "--sums", "s"])).is_err());
        assert!(
            parse(&args(&[
                "--version",
                "1.0.0",
                "--sums",
                "s",
                "--out",
                "o",
                "--repo",
                "x"
            ]))
            .is_err()
        );
        let options =
            parse(&args(&["--version", "1.0.0", "--sums", "s", "--out", "o"]))
                .unwrap();
        assert_eq!(options.repo, "cruisinme30/vellum");
    }

    #[test]
    fn formula_points_at_the_release() {
        let sums = sums();
        let release = Release {
            version: "1.2.3",
            repo: "cruisinme30/vellum",
            sums: &sums,
        };
        let text = formula(&release).unwrap();
        assert!(text.contains("class Vellum < Formula"));
        assert!(text.contains(
            "url \
                \"https://github.com/cruisinme30/vellum/releases/download/v1.2.\
                3/vellum-macos-universal\""
        ));
        assert!(text.contains(&format!("sha256 \"{}\"", digest(3))));
        assert!(text.contains("run [opt_bin/\"vellum\", \"serve\"]"));
        assert!(text.contains("#{bin}/vellum --version"));
        assert!(!text.contains("{{"));
    }

    #[test]
    fn winget_has_both_architectures() {
        let sums = sums();
        let release = Release {
            version: "1.2.3",
            repo: "cruisinme30/vellum",
            sums: &sums,
        };
        let files = winget(&release).unwrap();
        assert_eq!(files.len(), 3);
        assert!(files.iter().all(|(path, text)| {
            path.starts_with("winget/manifests/v/Vellum/Vellum/1.2.3")
                && text.contains("PackageVersion: 1.2.3\n")
                && text.ends_with("ManifestVersion: 1.9.0\n")
        }));
        let installer = &files[1].1;
        assert!(installer.contains("InstallerType: portable"));
        assert!(installer.contains("Architecture: arm64"));
        assert!(installer.contains(&digest(4).to_ascii_uppercase()));
    }

    #[test]
    fn a_missing_file_is_an_error() {
        let mut sums = sums();
        let _ = sums.remove(WINDOWS_ARM64);
        let release = Release {
            version: "1.2.3",
            repo: "cruisinme30/vellum",
            sums: &sums,
        };
        assert!(formula(&release).is_ok());
        assert_eq!(
            winget(&release).unwrap_err(),
            "SHA256SUMS has no vellum-windows-arm64.exe"
        );
    }
}
