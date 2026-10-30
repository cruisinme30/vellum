// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The fixed parts of a Word package: styles, settings and relationships.

use std::fmt::Write as _;

use super::xml;
use crate::flow::Media;

/// Namespaces declared on `w:document`.
pub const DOCUMENT_NAMESPACES: &str = concat!(
    r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" "#,
    concat!(
        r#"xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/rel"#,
        r#"ationships" "#,
    ),
    concat!(
        r#"xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordpro"#,
        r#"cessingDrawing" "#,
    ),
    r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" "#,
    r#"xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture" "#,
    r#"xmlns:asvg="http://schemas.microsoft.com/office/drawing/2016/SVG/main" "#,
    r#"xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math""#
);

/// `[Content_Types].xml` for the media types present.
pub fn content_types(media: &[Media]) -> String {
    let mut out = String::from(concat!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
        concat!(
            r#"<Types "#,
            r#"xmlns="http://schemas.openxmlformats.org/package/2006/content-"#,
            r#"types">"#,
        ),
        concat!(
            r#"<Default Extension="rels" "#,
            r#"ContentType="application/vnd.openxmlformats-package.relationsh"#,
            r#"ips+xml"/>"#,
        ),
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
        concat!(
            r#"<Override PartName="/word/document.xml" "#,
            r#"ContentType="application/vnd.openxmlformats-officedocument.wor"#,
            r#"dprocessingml.document.main+xml"/>"#,
        ),
        concat!(
            r#"<Override PartName="/word/styles.xml" "#,
            r#"ContentType="application/vnd.openxmlformats-officedocument.wor"#,
            r#"dprocessingml.styles+xml"/>"#,
        ),
        concat!(
            r#"<Override PartName="/word/numbering.xml" "#,
            r#"ContentType="application/vnd.openxmlformats-officedocument.wor"#,
            r#"dprocessingml.numbering+xml"/>"#,
        ),
        concat!(
            r#"<Override PartName="/word/settings.xml" "#,
            r#"ContentType="application/vnd.openxmlformats-officedocument.wor"#,
            r#"dprocessingml.settings+xml"/>"#,
        ),
        concat!(
            r#"<Override PartName="/word/footer1.xml" "#,
            r#"ContentType="application/vnd.openxmlformats-officedocument.wor"#,
            r#"dprocessingml.footer+xml"/>"#,
        ),
        concat!(
            r#"<Override PartName="/docProps/core.xml" "#,
            r#"ContentType="application/vnd.openxmlformats-package.core-prope"#,
            r#"rties+xml"/>"#,
        ),
        concat!(
            r#"<Override PartName="/docProps/app.xml" "#,
            r#"ContentType="application/vnd.openxmlformats-officedocument.ext"#,
            r#"ended-properties+xml"/>"#,
        ),
        "</Types>"
    ));
    out
}

/// `_rels/.rels`
pub const PACKAGE_RELS: &str = concat!(
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
    concat!(
        r#"<Relationships "#,
        r#"xmlns="http://schemas.openxmlformats.org/package/2006/relationship"#,
        r#"s">"#,
    ),
    concat!(
        r#"<Relationship Id="rId1" "#,
        r#"Type="http://schemas.openxmlformats.org/officeDocument/2006/relati"#,
        r#"onships/officeDocument" Target="word/document.xml"/>"#,
    ),
    concat!(
        r#"<Relationship Id="rId2" "#,
        r#"Type="http://schemas.openxmlformats.org/package/2006/relationships"#,
        r#"/metadata/core-properties" Target="docProps/core.xml"/>"#,
    ),
    concat!(
        r#"<Relationship Id="rId3" "#,
        r#"Type="http://schemas.openxmlformats.org/officeDocument/2006/relati"#,
        r#"onships/extended-properties" Target="docProps/app.xml"/>"#,
    ),
    "</Relationships>"
);

/// `docProps/core.xml`
pub fn core(title: &str, date: &str) -> String {
    format!(
        concat!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
            concat!(
                r#"<cp:coreProperties "#,
                r#"xmlns:cp="http://schemas.openxmlformats.org/package/2006/m"#,
                r#"etadata/core-properties" "#,
            ),
            concat!(
                r#"xmlns:dc="http://purl.org/dc/elements/1.1/" "#,
                r#"xmlns:dcterms="http://purl.org/dc/terms/" "#,
            ),
            r#"xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">"#,
            "<dc:title>{title}</dc:title><dc:creator>Vellum</dc:creator>",
            concat!(
                r#"<dcterms:created "#,
                r#"xsi:type="dcterms:W3CDTF">{date}T00:00:00Z</dcterms:create"#,
                r#"d>"#,
            ),
            "</cp:coreProperties>"
        ),
        title = xml(title),
        date = xml(date),
    )
}

/// `docProps/app.xml`
pub const APP: &str = concat!(
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
    concat!(
        r#"<Properties "#,
        r#"xmlns="http://schemas.openxmlformats.org/officeDocument/2006/exten"#,
        r#"ded-properties">"#,
    ),
    "<Application>Vellum</Application></Properties>"
);

/// `word/settings.xml`. `updateFields` makes Word refresh the contents
/// and the page numbers of cross-references when the file is opened.
pub const SETTINGS: &str = concat!(
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
    concat!(
        r#"<w:settings "#,
        r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/m"#,
        r#"ain" "#,
    ),
    r#"xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math">"#,
    r#"<w:updateFields w:val="true"/><w:defaultTabStop w:val="720"/>"#,
    r#"<w:characterSpacingControl w:val="doNotCompress"/>"#,
    r#"<m:mathPr><m:mathFont m:val="Cambria Math"/><m:dispDef/></m:mathPr>"#,
    concat!(
        r#"<w:compat><w:compatSetting w:name="compatibilityMode" "#,
        r#"w:uri="http://schemas.microsoft.com/office/word" "#,
        r#"w:val="15"/></w:compat>"#,
    ),
    "</w:settings>"
);

/// `word/footer1.xml`: centred page number.
pub const FOOTER: &str = concat!(
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
    concat!(
        r#"<w:ftr "#,
        r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/m"#,
        r#"ain">"#,
    ),
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
        concat!(
            r#"<w:numbering "#,
            r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/20"#,
            r#"06/main">"#,
        )
    ));
    let bullets = ["•", "◦", "▪"];
    for (abs, kind) in [(1, "bullet"), (2, "decimal"), (3, "none")] {
        let _ = write!(
            out,
            concat!(
                r#"<w:abstractNum w:abstractNumId="{abs}"><w:multiLevelType "#,
                r#"w:val="hybridMultilevel"/>"#,
            )
        );
        for level in 0..9u32 {
            let indent = 720 * (level + 1);
            let (fmt, text) = match kind {
                "bullet" => (
                    "bullet",
                    (*bullets.get(level as usize % 3).unwrap_or(&"•"))
                        .to_owned(),
                ),
                "decimal" => ("decimal", format!("%{}.", level + 1)),
                _ => ("none", String::new()),
            };
            let _ = write!(
                out,
                concat!(
                    concat!(
                        r#"<w:lvl w:ilvl="{level}"><w:start "#,
                        r#"w:val="1"/><w:numFmt w:val="{fmt}"/>"#,
                    ),
                    r#"<w:lvlText w:val="{text}"/><w:lvlJc w:val="left"/>"#,
                    concat!(
                        r#"<w:pPr><w:ind w:left="{indent}" "#,
                        r#"w:hanging="360"/></w:pPr></w:lvl>"#,
                    )
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
                concat!(
                    r#"<w:lvlOverride w:ilvl="0"><w:startOverride "#,
                    r#"w:val="{start}"/></w:lvlOverride>"#,
                )
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
            concat!(
                r#"<w:styles "#,
                r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingm"#,
                r#"l/2006/main">"#,
            ),
            concat!(
                r#"<w:docDefaults><w:rPrDefault><w:rPr><w:rFonts "#,
                r#"w:ascii="{body}" w:hAnsi="{body}" w:eastAsia="{body}" "#,
                r#"w:cs="{body}"/>"#,
            ),
            concat!(
                r#"<w:sz w:val="22"/><w:szCs w:val="22"/><w:lang "#,
                r#"w:val="en-GB"/></w:rPr></w:rPrDefault>"#,
            ),
            concat!(
                r#"<w:pPrDefault><w:pPr><w:spacing w:after="120" "#,
                r#"w:line="276" "#,
                r#"w:lineRule="auto"/></w:pPr></w:pPrDefault></w:docDefaults>"#,
            ),
            concat!(
                r#"<w:style w:type="paragraph" w:default="1" "#,
                r#"w:styleId="Normal"><w:name "#,
                r#"w:val="Normal"/><w:qFormat/></w:style>"#,
            ),
            concat!(
                r#"<w:style w:type="paragraph" w:styleId="Title"><w:name "#,
                r#"w:val="Title"/><w:basedOn w:val="Normal"/><w:next "#,
                r#"w:val="Normal"/><w:qFormat/>"#,
            ),
            concat!(
                r#"<w:pPr><w:spacing "#,
                r#"w:after="240"/></w:pPr><w:rPr><w:b/><w:color "#,
                r#"w:val="1D1B20"/><w:sz w:val="48"/></w:rPr></w:style>"#,
            ),
        ),
        body = xml(body_font)
    );
    let sizes = [36, 30, 26, 24, 22, 22];
    for (i, size) in sizes.iter().enumerate() {
        let level = i + 1;
        let _ = write!(
            out,
            concat!(
                concat!(
                    r#"<w:style w:type="paragraph" "#,
                    r#"w:styleId="Heading{level}"><w:name w:val="heading "#,
                    r#"{level}"/>"#,
                ),
                concat!(
                    r#"<w:basedOn w:val="Normal"/><w:next "#,
                    r#"w:val="Normal"/><w:qFormat/>"#,
                ),
                concat!(
                    r#"<w:pPr><w:keepNext/><w:keepLines/><w:spacing "#,
                    r#"w:before="{before}" w:after="120"/><w:outlineLvl "#,
                    r#"w:val="{lvl}"/></w:pPr>"#,
                ),
                concat!(
                    r#"<w:rPr><w:b/><w:color w:val="1D1B20"/><w:sz "#,
                    r#"w:val="{size}"/><w:szCs "#,
                    r#"w:val="{size}"/></w:rPr></w:style>"#,
                )
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
                concat!(
                    r#"<w:style w:type="paragraph" "#,
                    r#"w:styleId="TOC{level}"><w:name w:val="toc {level}"/>"#,
                ),
                r#"<w:basedOn w:val="Normal"/><w:next w:val="Normal"/>"#,
                concat!(
                    r#"<w:pPr><w:tabs><w:tab w:val="right" w:leader="dot" "#,
                    r#"w:pos="9350"/></w:tabs>"#,
                ),
                concat!(
                    r#"<w:spacing w:after="60"/><w:ind "#,
                    r#"w:left="{indent}"/></w:pPr></w:style>"#,
                )
            ),
            level = level,
            indent = (level - 1) * 240
        );
    }
    let _ = write!(
        out,
        concat!(
            concat!(
                r#"<w:style w:type="paragraph" "#,
                r#"w:styleId="TOCHeading"><w:name w:val="TOC "#,
                r#"Heading"/><w:basedOn w:val="Normal"/><w:next "#,
                r#"w:val="Normal"/>"#,
            ),
            concat!(
                r#"<w:pPr><w:keepNext/><w:spacing w:before="240" "#,
                r#"w:after="120"/></w:pPr><w:rPr><w:b/><w:sz "#,
                r#"w:val="30"/></w:rPr></w:style>"#,
            ),
            concat!(
                r#"<w:style w:type="paragraph" "#,
                r#"w:styleId="Subheading"><w:name "#,
                r#"w:val="Subheading"/><w:basedOn w:val="Normal"/><w:next "#,
                r#"w:val="Normal"/><w:qFormat/>"#,
            ),
            concat!(
                r#"<w:pPr><w:keepNext/><w:spacing w:before="200" "#,
                r#"w:after="80"/></w:pPr><w:rPr><w:b/><w:sz "#,
                r#"w:val="23"/></w:rPr></w:style>"#,
            ),
            concat!(
                r#"<w:style w:type="paragraph" w:styleId="Quote"><w:name "#,
                r#"w:val="Quote"/><w:basedOn w:val="Normal"/><w:next "#,
                r#"w:val="Normal"/><w:qFormat/>"#,
            ),
            concat!(
                r#"<w:pPr><w:pBdr><w:left w:val="single" w:sz="12" "#,
                r#"w:space="8" w:color="C5C6D0"/></w:pBdr><w:ind "#,
                r#"w:left="360"/></w:pPr>"#,
            ),
            r#"<w:rPr><w:i/><w:color w:val="46464F"/></w:rPr></w:style>"#,
            concat!(
                r#"<w:style w:type="paragraph" w:styleId="Code"><w:name "#,
                r#"w:val="Code"/><w:basedOn w:val="Normal"/><w:qFormat/>"#,
            ),
            concat!(
                r#"<w:pPr><w:shd w:val="clear" w:color="auto" "#,
                r#"w:fill="F0F2F9"/><w:spacing w:after="160" w:line="240" "#,
                r#"w:lineRule="auto"/></w:pPr>"#,
            ),
            concat!(
                r#"<w:rPr><w:rFonts w:ascii="{mono}" w:hAnsi="{mono}" "#,
                r#"w:cs="{mono}"/><w:sz w:val="19"/></w:rPr></w:style>"#,
            ),
            concat!(
                r#"<w:style w:type="paragraph" w:styleId="Caption"><w:name "#,
                r#"w:val="caption"/><w:basedOn w:val="Normal"/><w:next "#,
                r#"w:val="Normal"/><w:qFormat/>"#,
            ),
            concat!(
                r#"<w:pPr><w:jc w:val="center"/><w:spacing "#,
                r#"w:after="200"/></w:pPr><w:rPr><w:i/><w:color "#,
                r#"w:val="46464F"/><w:sz w:val="19"/></w:rPr></w:style>"#,
            ),
            concat!(
                r#"<w:style w:type="paragraph" "#,
                r#"w:styleId="ListParagraph"><w:name w:val="List "#,
                r#"Paragraph"/><w:basedOn w:val="Normal"/><w:qFormat/>"#,
            ),
            concat!(
                r#"<w:pPr><w:spacing w:after="60"/><w:ind "#,
                r#"w:left="720"/><w:contextualSpacing/></w:pPr></w:style>"#,
            ),
            concat!(
                r#"<w:style w:type="paragraph" w:styleId="Footer"><w:name "#,
                r#"w:val="footer"/><w:basedOn w:val="Normal"/><w:rPr><w:sz "#,
                r#"w:val="18"/><w:color w:val="46464F"/></w:rPr></w:style>"#,
            ),
            concat!(
                r#"<w:style w:type="character" w:styleId="Hyperlink"><w:name "#,
                r#"w:val="Hyperlink"/><w:rPr><w:color "#,
                r#"w:val="{primary}"/><w:u w:val="single"/></w:rPr></w:style>"#,
            ),
            concat!(
                r#"<w:style w:type="character" w:styleId="CodeChar"><w:name "#,
                r#"w:val="Inline Code"/>"#,
            ),
            concat!(
                r#"<w:rPr><w:rFonts w:ascii="{mono}" w:hAnsi="{mono}" "#,
                r#"w:cs="{mono}"/><w:sz w:val="20"/><w:shd w:val="clear" "#,
                r#"w:color="auto" w:fill="F0F2F9"/></w:rPr></w:style>"#,
            ),
            concat!(
                r#"<w:style w:type="table" w:default="1" "#,
                r#"w:styleId="TableNormal"><w:name w:val="Normal Table"/>"#,
            ),
            concat!(
                r#"<w:tblPr><w:tblInd w:w="0" "#,
                r#"w:type="dxa"/><w:tblCellMar><w:top w:w="0" "#,
                r#"w:type="dxa"/><w:left w:w="108" w:type="dxa"/>"#,
            ),
            concat!(
                r#"<w:bottom w:w="0" w:type="dxa"/><w:right w:w="108" "#,
                r#"w:type="dxa"/></w:tblCellMar></w:tblPr></w:style>"#,
            ),
            concat!(
                r#"<w:style w:type="table" w:styleId="VellumTable"><w:name "#,
                r#"w:val="Vellum Table"/><w:basedOn w:val="TableNormal"/>"#,
            ),
            r#"<w:pPr><w:spacing w:before="40" w:after="40"/></w:pPr>"#,
            concat!(
                r#"<w:tblPr><w:tblBorders><w:top w:val="single" w:sz="4" "#,
                r#"w:color="C5C6D0"/><w:left w:val="single" w:sz="4" "#,
                r#"w:color="C5C6D0"/>"#,
            ),
            concat!(
                r#"<w:bottom w:val="single" w:sz="4" "#,
                r#"w:color="C5C6D0"/><w:right w:val="single" w:sz="4" "#,
                r#"w:color="C5C6D0"/>"#,
            ),
            concat!(
                r#"<w:insideH w:val="single" w:sz="4" "#,
                r#"w:color="C5C6D0"/><w:insideV w:val="single" w:sz="4" "#,
                r#"w:color="C5C6D0"/></w:tblBorders>"#,
            ),
            concat!(
                r#"<w:tblCellMar><w:top w:w="60" w:type="dxa"/><w:left "#,
                r#"w:w="100" w:type="dxa"/><w:bottom w:w="60" "#,
                r#"w:type="dxa"/><w:right w:w="100" "#,
                r#"w:type="dxa"/></w:tblCellMar></w:tblPr></w:style>"#,
            ),
            concat!(
                r#"<w:style w:type="table" w:styleId="VellumPlain"><w:name "#,
                r#"w:val="Vellum Plain"/><w:basedOn w:val="TableNormal"/>"#,
            ),
            concat!(
                r#"<w:tblPr><w:tblCellMar><w:top w:w="40" "#,
                r#"w:type="dxa"/><w:left w:w="100" w:type="dxa"/><w:bottom "#,
                r#"w:w="40" w:type="dxa"/><w:right w:w="100" "#,
                r#"w:type="dxa"/></w:tblCellMar></w:tblPr></w:style>"#,
            ),
            "</w:styles>"
        ),
        mono = xml(mono_font),
        primary = xml(primary.trim_start_matches('#'))
    );
    out
}
