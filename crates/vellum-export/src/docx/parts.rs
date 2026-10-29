// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The fixed parts of a Word package: styles, settings and relationships.

use std::fmt::Write as _;

use super::xml;
use crate::flow::Media;

/// Namespaces declared on `w:document`.
pub const DOCUMENT_NAMESPACES: &str = concat!(
    r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" "#,
    r#"xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" "#,
    r#"xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" "#,
    r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" "#,
    r#"xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture" "#,
    r#"xmlns:asvg="http://schemas.microsoft.com/office/drawing/2016/SVG/main" "#,
    r#"xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math""#
);

/// `[Content_Types].xml` for the media types present.
pub fn content_types(media: &[Media]) -> String {
    let mut out = String::from(concat!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
        r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">"#,
        r#"<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>"#,
        r#"<Default Extension="xml" ContentType="application/xml"/>"#,
    ));
    let mut seen = Vec::new();
    for m in media {
        if !seen.contains(m) {
            seen.push(*m);
            let _ = write!(
                out,
                r#"<Default Extension="{}" ContentType="{}"/>"#,
                m.extension(),
                m.mime()
            );
        }
    }
    out.push_str(concat!(
        r#"<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>"#,
        r#"<Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/>"#,
        r#"<Override PartName="/word/numbering.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml"/>"#,
        r#"<Override PartName="/word/settings.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml"/>"#,
        r#"<Override PartName="/word/footer1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml"/>"#,
        r#"<Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/>"#,
        r#"<Override PartName="/docProps/app.xml" ContentType="application/vnd.openxmlformats-officedocument.extended-properties+xml"/>"#,
        "</Types>"
    ));
    out
}

/// `_rels/.rels`
pub const PACKAGE_RELS: &str = concat!(
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
    r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
    r#"<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>"#,
    r#"<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/>"#,
    r#"<Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties" Target="docProps/app.xml"/>"#,
    "</Relationships>"
);

/// `docProps/core.xml`
pub fn core(title: &str, date: &str) -> String {
    format!(
        concat!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
            r#"<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" "#,
            r#"xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" "#,
            r#"xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">"#,
            "<dc:title>{title}</dc:title><dc:creator>Vellum</dc:creator>",
            r#"<dcterms:created xsi:type="dcterms:W3CDTF">{date}T00:00:00Z</dcterms:created>"#,
            "</cp:coreProperties>"
        ),
        title = xml(title),
        date = xml(date),
    )
}

/// `docProps/app.xml`
pub const APP: &str = concat!(
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
    r#"<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties">"#,
    "<Application>Vellum</Application></Properties>"
);

/// `word/settings.xml`. `updateFields` makes Word refresh the contents
/// and the page numbers of cross-references when the file is opened.
pub const SETTINGS: &str = concat!(
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
    r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" "#,
    r#"xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math">"#,
    r#"<w:updateFields w:val="true"/><w:defaultTabStop w:val="720"/>"#,
    r#"<w:characterSpacingControl w:val="doNotCompress"/>"#,
    r#"<m:mathPr><m:mathFont m:val="Cambria Math"/><m:dispDef/></m:mathPr>"#,
    r#"<w:compat><w:compatSetting w:name="compatibilityMode" w:uri="http://schemas.microsoft.com/office/word" w:val="15"/></w:compat>"#,
    "</w:settings>"
);

/// `word/footer1.xml`: centred page number.
pub const FOOTER: &str = concat!(
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
    r#"<w:ftr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">"#,
    r#"<w:p><w:pPr><w:pStyle w:val="Footer"/><w:jc w:val="center"/></w:pPr>"#,
    r#"<w:r><w:fldChar w:fldCharType="begin"/></w:r>"#,
    r#"<w:r><w:instrText xml:space="preserve"> PAGE </w:instrText></w:r>"#,
    r#"<w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>1</w:t></w:r>"#,
    r#"<w:r><w:fldChar w:fldCharType="end"/></w:r></w:p></w:ftr>"#
);

/// `word/numbering.xml`: abstract 1 is bullets, abstract 2 is decimal,
/// abstract 3 is unmarked (task lists, whose items carry ☑ or ☐).
/// Each list gets its own `w:num` so numbered lists restart.
pub fn numbering(nums: &[(u32, u32, u64)]) -> String {
    let mut out = String::from(concat!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
        r#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">"#
    ));
    let bullets = ["•", "◦", "▪"];
    for (abs, kind) in [(1, "bullet"), (2, "decimal"), (3, "none")] {
        let _ = write!(
            out,
            r#"<w:abstractNum w:abstractNumId="{abs}"><w:multiLevelType w:val="hybridMultilevel"/>"#
        );
        for level in 0..9u32 {
            let indent = 720 * (level + 1);
            let (fmt, text) = match kind {
                "bullet" => (
                    "bullet",
                    (*bullets.get(level as usize % 3).unwrap_or(&"•")).to_owned(),
                ),
                "decimal" => ("decimal", format!("%{}.", level + 1)),
                _ => ("none", String::new()),
            };
            let _ = write!(
                out,
                concat!(
                    r#"<w:lvl w:ilvl="{level}"><w:start w:val="1"/><w:numFmt w:val="{fmt}"/>"#,
                    r#"<w:lvlText w:val="{text}"/><w:lvlJc w:val="left"/>"#,
                    r#"<w:pPr><w:ind w:left="{indent}" w:hanging="360"/></w:pPr></w:lvl>"#
                ),
                level = level,
                fmt = fmt,
                text = text,
                indent = indent
            );
        }
        out.push_str("</w:abstractNum>");
    }
    for (num, abs, start) in nums {
        let _ = write!(
            out,
            r#"<w:num w:numId="{num}"><w:abstractNumId w:val="{abs}"/>"#
        );
        if *abs == 2 {
            let _ = write!(
                out,
                r#"<w:lvlOverride w:ilvl="0"><w:startOverride w:val="{start}"/></w:lvlOverride>"#
            );
        }
        out.push_str("</w:num>");
    }
    out.push_str("</w:numbering>");
    out
}

/// `word/styles.xml`. Headings carry outline levels so the navigation
/// pane and Google Docs' outline work.
pub fn styles(body_font: &str, mono_font: &str, primary: &str) -> String {
    let mut out = format!(
        concat!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
            r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">"#,
            r#"<w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="{body}" w:hAnsi="{body}" w:eastAsia="{body}" w:cs="{body}"/>"#,
            r#"<w:sz w:val="22"/><w:szCs w:val="22"/><w:lang w:val="en-GB"/></w:rPr></w:rPrDefault>"#,
            r#"<w:pPrDefault><w:pPr><w:spacing w:after="120" w:line="276" w:lineRule="auto"/></w:pPr></w:pPrDefault></w:docDefaults>"#,
            r#"<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:qFormat/></w:style>"#,
            r#"<w:style w:type="paragraph" w:styleId="Title"><w:name w:val="Title"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:qFormat/>"#,
            r#"<w:pPr><w:spacing w:after="240"/></w:pPr><w:rPr><w:b/><w:color w:val="1D1B20"/><w:sz w:val="48"/></w:rPr></w:style>"#,
        ),
        body = xml(body_font)
    );
    let sizes = [36, 30, 26, 24, 22, 22];
    for (i, size) in sizes.iter().enumerate() {
        let level = i + 1;
        let _ = write!(
            out,
            concat!(
                r#"<w:style w:type="paragraph" w:styleId="Heading{level}"><w:name w:val="heading {level}"/>"#,
                r#"<w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:qFormat/>"#,
                r#"<w:pPr><w:keepNext/><w:keepLines/><w:spacing w:before="{before}" w:after="120"/><w:outlineLvl w:val="{lvl}"/></w:pPr>"#,
                r#"<w:rPr><w:b/><w:color w:val="1D1B20"/><w:sz w:val="{size}"/><w:szCs w:val="{size}"/></w:rPr></w:style>"#
            ),
            level = level,
            lvl = i,
            before = if level <= 2 { 360 } else { 240 },
            size = size
        );
    }
    for level in 1..=3 {
        let _ = write!(
            out,
            concat!(
                r#"<w:style w:type="paragraph" w:styleId="TOC{level}"><w:name w:val="toc {level}"/>"#,
                r#"<w:basedOn w:val="Normal"/><w:next w:val="Normal"/>"#,
                r#"<w:pPr><w:tabs><w:tab w:val="right" w:leader="dot" w:pos="9350"/></w:tabs>"#,
                r#"<w:spacing w:after="60"/><w:ind w:left="{indent}"/></w:pPr></w:style>"#
            ),
            level = level,
            indent = (level - 1) * 240
        );
    }
    let _ = write!(
        out,
        concat!(
            r#"<w:style w:type="paragraph" w:styleId="TOCHeading"><w:name w:val="TOC Heading"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/>"#,
            r#"<w:pPr><w:keepNext/><w:spacing w:before="240" w:after="120"/></w:pPr><w:rPr><w:b/><w:sz w:val="30"/></w:rPr></w:style>"#,
            r#"<w:style w:type="paragraph" w:styleId="Subheading"><w:name w:val="Subheading"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:qFormat/>"#,
            r#"<w:pPr><w:keepNext/><w:spacing w:before="200" w:after="80"/></w:pPr><w:rPr><w:b/><w:sz w:val="23"/></w:rPr></w:style>"#,
            r#"<w:style w:type="paragraph" w:styleId="Quote"><w:name w:val="Quote"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:qFormat/>"#,
            r#"<w:pPr><w:pBdr><w:left w:val="single" w:sz="12" w:space="8" w:color="C5C6D0"/></w:pBdr><w:ind w:left="360"/></w:pPr>"#,
            r#"<w:rPr><w:i/><w:color w:val="46464F"/></w:rPr></w:style>"#,
            r#"<w:style w:type="paragraph" w:styleId="Code"><w:name w:val="Code"/><w:basedOn w:val="Normal"/><w:qFormat/>"#,
            r#"<w:pPr><w:shd w:val="clear" w:color="auto" w:fill="F0F2F9"/><w:spacing w:after="160" w:line="240" w:lineRule="auto"/></w:pPr>"#,
            r#"<w:rPr><w:rFonts w:ascii="{mono}" w:hAnsi="{mono}" w:cs="{mono}"/><w:sz w:val="19"/></w:rPr></w:style>"#,
            r#"<w:style w:type="paragraph" w:styleId="Caption"><w:name w:val="caption"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:qFormat/>"#,
            r#"<w:pPr><w:jc w:val="center"/><w:spacing w:after="200"/></w:pPr><w:rPr><w:i/><w:color w:val="46464F"/><w:sz w:val="19"/></w:rPr></w:style>"#,
            r#"<w:style w:type="paragraph" w:styleId="ListParagraph"><w:name w:val="List Paragraph"/><w:basedOn w:val="Normal"/><w:qFormat/>"#,
            r#"<w:pPr><w:spacing w:after="60"/><w:ind w:left="720"/><w:contextualSpacing/></w:pPr></w:style>"#,
            r#"<w:style w:type="paragraph" w:styleId="Footer"><w:name w:val="footer"/><w:basedOn w:val="Normal"/><w:rPr><w:sz w:val="18"/><w:color w:val="46464F"/></w:rPr></w:style>"#,
            r#"<w:style w:type="character" w:styleId="Hyperlink"><w:name w:val="Hyperlink"/><w:rPr><w:color w:val="{primary}"/><w:u w:val="single"/></w:rPr></w:style>"#,
            r#"<w:style w:type="character" w:styleId="CodeChar"><w:name w:val="Inline Code"/>"#,
            r#"<w:rPr><w:rFonts w:ascii="{mono}" w:hAnsi="{mono}" w:cs="{mono}"/><w:sz w:val="20"/><w:shd w:val="clear" w:color="auto" w:fill="F0F2F9"/></w:rPr></w:style>"#,
            r#"<w:style w:type="table" w:default="1" w:styleId="TableNormal"><w:name w:val="Normal Table"/>"#,
            r#"<w:tblPr><w:tblInd w:w="0" w:type="dxa"/><w:tblCellMar><w:top w:w="0" w:type="dxa"/><w:left w:w="108" w:type="dxa"/>"#,
            r#"<w:bottom w:w="0" w:type="dxa"/><w:right w:w="108" w:type="dxa"/></w:tblCellMar></w:tblPr></w:style>"#,
            r#"<w:style w:type="table" w:styleId="VellumTable"><w:name w:val="Vellum Table"/><w:basedOn w:val="TableNormal"/>"#,
            r#"<w:pPr><w:spacing w:before="40" w:after="40"/></w:pPr>"#,
            r#"<w:tblPr><w:tblBorders><w:top w:val="single" w:sz="4" w:color="C5C6D0"/><w:left w:val="single" w:sz="4" w:color="C5C6D0"/>"#,
            r#"<w:bottom w:val="single" w:sz="4" w:color="C5C6D0"/><w:right w:val="single" w:sz="4" w:color="C5C6D0"/>"#,
            r#"<w:insideH w:val="single" w:sz="4" w:color="C5C6D0"/><w:insideV w:val="single" w:sz="4" w:color="C5C6D0"/></w:tblBorders>"#,
            r#"<w:tblCellMar><w:top w:w="60" w:type="dxa"/><w:left w:w="100" w:type="dxa"/><w:bottom w:w="60" w:type="dxa"/><w:right w:w="100" w:type="dxa"/></w:tblCellMar></w:tblPr></w:style>"#,
            r#"<w:style w:type="table" w:styleId="VellumPlain"><w:name w:val="Vellum Plain"/><w:basedOn w:val="TableNormal"/>"#,
            r#"<w:tblPr><w:tblCellMar><w:top w:w="40" w:type="dxa"/><w:left w:w="100" w:type="dxa"/><w:bottom w:w="40" w:type="dxa"/><w:right w:w="100" w:type="dxa"/></w:tblCellMar></w:tblPr></w:style>"#,
            "</w:styles>"
        ),
        mono = xml(mono_font),
        primary = xml(primary.trim_start_matches('#'))
    );
    out
}
