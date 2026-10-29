// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Mermaid, rendered in-process by the bundled pure-Rust engine (merman).
//!
//! Diagrams are drawn with Mermaid's `base` theme whose variables are the
//! Light theme's colours; afterwards every exact use of one of those
//! colours is replaced by the matching `var(--vl-…)`, so the main fills,
//! borders and text of every diagram type follow the page theme (R-TH-3).

use std::sync::LazyLock;
use std::time::Duration;

use merman::{OperationControl, RenderOutput, RenderRequest, Renderer, SvgRequest};

use crate::theme::{BuiltinTheme, Theme};

/// Mermaid theme variable → Vellum token.
const VARIABLES: &[(&str, &str)] = &[
    ("primaryColor", "primary-container"),
    ("primaryTextColor", "on-primary-container"),
    ("primaryBorderColor", "primary"),
    ("secondaryColor", "secondary-container"),
    ("secondaryTextColor", "on-secondary-container"),
    ("secondaryBorderColor", "secondary"),
    ("tertiaryColor", "tertiary-container"),
    ("tertiaryTextColor", "on-tertiary-container"),
    ("tertiaryBorderColor", "tertiary"),
    ("background", "surface"),
    ("mainBkg", "primary-container"),
    ("nodeBorder", "primary"),
    ("nodeTextColor", "on-primary-container"),
    ("lineColor", "on-surface-variant"),
    ("textColor", "on-surface"),
    ("titleColor", "on-surface"),
    ("clusterBkg", "surface-container-low"),
    ("clusterBorder", "outline-variant"),
    ("edgeLabelBackground", "surface-container"),
    ("noteBkgColor", "warn-container"),
    ("noteTextColor", "on-warn-container"),
    ("noteBorderColor", "warn"),
    ("actorBkg", "primary-container"),
    ("actorBorder", "primary"),
    ("actorTextColor", "on-primary-container"),
    ("actorLineColor", "outline"),
    ("signalColor", "on-surface"),
    ("signalTextColor", "on-surface"),
    ("labelBoxBkgColor", "surface-container"),
    ("labelBoxBorderColor", "outline"),
    ("labelTextColor", "on-surface"),
    ("loopTextColor", "on-surface"),
    ("activationBkgColor", "surface-container-high"),
    ("activationBorderColor", "outline"),
    ("errorBkgColor", "error-container"),
    ("errorTextColor", "on-error-container"),
    ("pie1", "chart-1"),
    ("pie2", "chart-2"),
    ("pie3", "chart-3"),
    ("pie4", "chart-4"),
    ("pie5", "chart-5"),
    ("pie6", "chart-6"),
    ("pie7", "chart-7"),
    ("pie8", "chart-8"),
    ("pieTitleTextColor", "on-surface"),
    ("pieLegendTextColor", "on-surface"),
    ("pieStrokeColor", "surface"),
    ("git0", "chart-1"),
    ("git1", "chart-2"),
    ("git2", "chart-3"),
    ("git3", "chart-4"),
    ("git4", "chart-5"),
    ("git5", "chart-6"),
    ("git6", "chart-7"),
    ("git7", "chart-8"),
    ("cScale0", "chart-1"),
    ("cScale1", "chart-2"),
    ("cScale2", "chart-3"),
    ("cScale3", "chart-4"),
    ("cScale4", "chart-5"),
    ("cScale5", "chart-6"),
    ("cScale6", "chart-7"),
    ("cScale7", "chart-8"),
];

struct Palette {
    /// The `%%{init}%%` directive prepended to every diagram.
    directive: String,
    /// Lower-case `#rrggbb` → token, first mapping wins.
    reverse: Vec<(String, &'static str)>,
}

static PALETTE: LazyLock<Palette> = LazyLock::new(|| {
    let light = Theme::builtin(BuiltinTheme::Light);
    let mut vars = Vec::new();
    let mut reverse: Vec<(String, &'static str)> = Vec::new();
    for (var, token) in VARIABLES {
        let hex = light.get(token).to_string();
        vars.push(format!("\"{var}\":\"{hex}\""));
        if !reverse.iter().any(|(h, _)| *h == hex) {
            reverse.push((hex, token));
        }
    }
    vars.push("\"fontFamily\":\"Roboto, system-ui, sans-serif\"".to_owned());
    let directive = format!(
        "%%{{init: {{\"theme\":\"base\",\"themeVariables\":{{{}}}}}}}%%\n",
        vars.join(",")
    );
    Palette { directive, reverse }
});

/// Renders Mermaid source to a themed SVG with `var(--vl-…)` colours.
///
/// # Errors
///
/// Returns Mermaid's message when the source does not parse, names no known
/// diagram type, or runs past `deadline`.
pub fn render(source: &str, deadline: Duration) -> Result<String, String> {
    if source.len() > 64 * 1024 {
        return Err("diagram source is longer than 64 KiB".to_owned());
    }
    let palette = &*PALETTE;
    let themed = format!("{}{source}", palette.directive);
    let control = OperationControl::new().with_deadline(deadline);
    let output = Renderer::new()
        .render(RenderRequest::svg(&themed, control, SvgRequest::default()))
        .map_err(|e| e.to_string())?;
    let svg = match output {
        RenderOutput::Svg(Some(svg)) => svg.svg().to_owned(),
        _ => return Err("no Mermaid diagram detected".to_owned()),
    };
    Ok(retheme(&svg, &palette.reverse))
}

/// Replaces palette colours with CSS variables. A `#rrggbb` that is part of
/// a fragment reference (`url(#…)`, `href="#…"`) is left alone.
fn retheme(svg: &str, reverse: &[(String, &str)]) -> String {
    let mut out = String::with_capacity(svg.len() + svg.len() / 8);
    let bytes = svg.as_bytes();
    let mut i = 0;
    let mut last = 0;
    while i < bytes.len() {
        if bytes.get(i) == Some(&b'#') {
            let hex = svg.get(i + 1..i + 7);
            let boundary = bytes
                .get(i + 7)
                .is_none_or(|b| !b.is_ascii_alphanumeric() && *b != b'-' && *b != b'_');
            let prev = if i == 0 { None } else { bytes.get(i - 1) };
            let is_ref = prev == Some(&b'(') || svg.get(i.saturating_sub(6)..i) == Some("href=\"");
            if let Some(hex) =
                hex.filter(|h| boundary && !is_ref && h.bytes().all(|b| b.is_ascii_hexdigit()))
            {
                let lower = format!("#{}", hex.to_ascii_lowercase());
                if let Some((_, token)) = reverse.iter().find(|(h, _)| *h == lower) {
                    out.push_str(svg.get(last..i).unwrap_or_default());
                    out.push_str("var(--vl-");
                    out.push_str(token);
                    out.push(')');
                    i += 7;
                    last = i;
                    continue;
                }
            }
        }
        i += 1;
    }
    out.push_str(svg.get(last..).unwrap_or_default());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sanitize::xml::sanitize_island;

    const DEADLINE: Duration = Duration::from_secs(10);

    #[test]
    fn renders_and_rethemes_a_flowchart() {
        let svg = render(
            "flowchart LR\n  A[SDK client] --> B{Gateway}\n  B -->|allowed| C[Backend]\n  B -->|blocked| D[(Audit log)]\n",
            DEADLINE,
        )
        .unwrap();
        assert!(svg.contains("SDK client"), "{svg}");
        assert!(svg.contains("var(--vl-"), "no themed colours in {svg}");
        let clean = sanitize_island(&svg, "f1").unwrap();
        assert!(clean.contains("Gateway"));
        assert!(!clean.contains("<script"));
    }

    #[test]
    fn renders_common_diagram_types() {
        for src in [
            "sequenceDiagram\n  Browser->>Gateway: open page\n  Gateway-->>Browser: page\n",
            "pie title Pets\n  \"Dogs\" : 386\n  \"Cats\" : 85\n",
            "stateDiagram-v2\n  [*] --> Draft\n  Draft --> Verified\n",
            "classDiagram\n  class Page {\n    +String title\n  }\n",
            "erDiagram\n  PAGE ||--o{ BLOCK : contains\n",
            "gantt\n  dateFormat YYYY-MM-DD\n  section Build\n  Parser :a1, 2026-11-01, 30d\n",
            "gitGraph\n  commit\n  branch develop\n  commit\n",
            "mindmap\n  root((Vellum))\n    Docs\n    Work\n",
        ] {
            let svg = render(src, DEADLINE).unwrap_or_else(|e| panic!("{src}: {e}"));
            assert!(sanitize_island(&svg, "f").is_ok(), "{src}");
        }
    }

    #[test]
    fn reports_bad_source() {
        assert!(render("not a diagram at all", DEADLINE).is_err());
        assert!(render(&"x".repeat(70_000), DEADLINE).is_err());
    }

    #[test]
    fn retheme_leaves_references_alone() {
        let reverse = vec![("#abcdef".to_owned(), "primary")];
        let out = retheme(
            r##"<p fill="#ABCDEF" style="stroke:#abcdef;" marker="url(#abcdef)" href="#abcdef"/><q fill="#abcdef0"/>"##,
            &reverse,
        );
        assert_eq!(
            out,
            r##"<p fill="var(--vl-primary)" style="stroke:var(--vl-primary);" marker="url(#abcdef)" href="#abcdef"/><q fill="#abcdef0"/>"##
        );
    }
}
