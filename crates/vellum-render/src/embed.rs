// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Sandboxed embeds (`::: embed <url>`).
//!
//! Only well-known providers are framed, always through their dedicated
//! embed endpoint over https, inside a sandboxed `<iframe>` with no
//! referrer. Any other URL renders as a plain link card, so a page can never
//! frame an arbitrary site.

use std::fmt::Write as _;

/// A provider that may be framed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    /// YouTube, via the privacy-enhanced `youtube-nocookie.com` player.
    YouTube,
    /// Vimeo.
    Vimeo,
    /// Loom.
    Loom,
    /// Figma.
    Figma,
    /// Google Maps.
    GoogleMaps,
    /// CodePen.
    CodePen,
}

impl Provider {
    /// The provider's display name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::YouTube => "YouTube",
            Self::Vimeo => "Vimeo",
            Self::Loom => "Loom",
            Self::Figma => "Figma",
            Self::GoogleMaps => "Google Maps",
            Self::CodePen => "CodePen",
        }
    }
}

/// Hosts whose embed endpoints may appear in an `<iframe src>`.
const FRAME_PREFIXES: &[&str] = &[
    "https://www.youtube-nocookie.com/embed/",
    "https://player.vimeo.com/video/",
    "https://www.loom.com/embed/",
    "https://www.figma.com/embed?",
    "https://www.google.com/maps/embed",
    "https://codepen.io/",
];

/// True when `src` is an embed URL this module produces.
#[must_use]
pub fn is_allowed_frame_src(src: &str) -> bool {
    FRAME_PREFIXES.iter().any(|p| src.starts_with(p)) && !src.contains(['"', '<', '>', ' ', '\\'])
}

/// Splits `https://host/path?query` into its host and the rest.
fn host_and_path(url: &str) -> Option<(&str, &str)> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let host = rest.get(..end)?;
    let path = rest.get(end..).unwrap_or_default();
    Some((host, path))
}

fn query_param<'a>(path: &'a str, key: &str) -> Option<&'a str> {
    let query = path.split_once('?')?.1;
    let query = query.split('#').next().unwrap_or_default();
    query.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k == key).then_some(v)
    })
}

fn is_token(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// The provider and frame URL for a page URL, or `None` when the URL is not
/// an embeddable page of a known provider.
#[must_use]
pub fn frame_src(url: &str) -> Option<(Provider, String)> {
    let url = url.trim();
    let (host, path) = host_and_path(url)?;
    let host = host.to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    let first = |p: &str| {
        p.trim_start_matches('/')
            .split(['/', '?', '#'])
            .next()
            .map(str::to_owned)
    };
    match host {
        "youtube.com" | "m.youtube.com" => {
            let id = query_param(path, "v")
                .map(str::to_owned)
                .or_else(|| path.strip_prefix("/shorts/").and_then(first))
                .or_else(|| path.strip_prefix("/embed/").and_then(first))?;
            is_token(&id).then(|| {
                (
                    Provider::YouTube,
                    format!("https://www.youtube-nocookie.com/embed/{id}"),
                )
            })
        }
        "youtu.be" => {
            let id = first(path)?;
            is_token(&id).then(|| {
                (
                    Provider::YouTube,
                    format!("https://www.youtube-nocookie.com/embed/{id}"),
                )
            })
        }
        "vimeo.com" => {
            let id = first(path)?;
            (id.chars().all(|c| c.is_ascii_digit()) && !id.is_empty()).then(|| {
                (
                    Provider::Vimeo,
                    format!("https://player.vimeo.com/video/{id}"),
                )
            })
        }
        "loom.com" => {
            let id = path.strip_prefix("/share/").and_then(first)?;
            is_token(&id).then(|| (Provider::Loom, format!("https://www.loom.com/embed/{id}")))
        }
        "figma.com" => {
            let kind = first(path)?;
            matches!(kind.as_str(), "file" | "design" | "proto" | "board").then(|| {
                let target = format!(
                    "https://www.figma.com{}",
                    path.split('#').next().unwrap_or_default()
                );
                (
                    Provider::Figma,
                    format!(
                        "https://www.figma.com/embed?embed_host=vellum&url={}",
                        percent_encode(&target)
                    ),
                )
            })
        }
        "google.com" if path.starts_with("/maps/embed") => Some((
            Provider::GoogleMaps,
            format!("https://www.google.com{path}"),
        ))
        .filter(|(_, s)| is_allowed_frame_src(s)),
        "codepen.io" => {
            let mut parts = path.trim_start_matches('/').split('/');
            let user = parts.next()?;
            let kind = parts.next()?;
            let id = parts.next()?.split(['?', '#']).next()?;
            (is_token(user) && matches!(kind, "pen" | "embed") && is_token(id)).then(|| {
                (
                    Provider::CodePen,
                    format!("https://codepen.io/{user}/embed/{id}?default-tab=result"),
                )
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_known_providers_to_embed_endpoints() {
        let cases = [
            (
                "https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=1",
                "https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ",
            ),
            (
                "https://youtu.be/dQw4w9WgXcQ",
                "https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ",
            ),
            (
                "https://vimeo.com/76979871",
                "https://player.vimeo.com/video/76979871",
            ),
            (
                "https://www.loom.com/share/abc123",
                "https://www.loom.com/embed/abc123",
            ),
            (
                "https://codepen.io/northstar/pen/xYzAb",
                "https://codepen.io/northstar/embed/xYzAb?default-tab=result",
            ),
        ];
        for (url, expected) in cases {
            let (_, src) = frame_src(url).unwrap();
            assert_eq!(src, expected);
            assert!(is_allowed_frame_src(&src));
        }
        let (p, src) = frame_src("https://www.figma.com/design/KEY/Name").unwrap();
        assert_eq!(p, Provider::Figma);
        assert!(
            src.starts_with("https://www.figma.com/embed?embed_host=vellum&url=https%3A%2F%2F")
        );
    }

    #[test]
    fn refuses_everything_else() {
        for url in [
            "https://northstar.dev/page",
            "javascript:alert(1)",
            "https://www.youtube.com/watch?v=\"><script>",
            "https://youtube.com.evil.example/watch?v=abc",
            "https://vimeo.com/abc",
            "ftp://youtube.com/watch?v=abc",
        ] {
            assert!(frame_src(url).is_none(), "{url}");
        }
        assert!(!is_allowed_frame_src("https://evil.example/embed"));
    }
}
