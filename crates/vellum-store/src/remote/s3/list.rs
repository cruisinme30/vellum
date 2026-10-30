// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! A small extractor for `ListObjectsV2` responses.
//!
//! The response is flat and machine-written, so the adapter reads only the
//! elements it needs instead of pulling in an XML parser.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::sigv4::days_from_civil;

/// One `<Contents>` entry.
#[derive(Debug, PartialEq, Eq)]
pub struct Object {
    /// The full object key, decoded.
    pub key: String,
    /// Size in bytes.
    pub size: u64,
    /// `LastModified`, or the epoch when missing or malformed.
    pub modified: SystemTime,
}

/// One page of a listing.
#[derive(Debug, PartialEq, Eq)]
pub struct Page {
    /// The objects on this page. A key that cannot be decoded is `None`.
    pub objects: Vec<Option<Object>>,
    /// The token for the next page, when the listing is truncated.
    pub next: Option<String>,
}

/// Parses a `ListObjectsV2` response body.
///
/// When the body says `<EncodingType>url</EncodingType>`, keys are
/// URL-decoded as well as XML-unescaped.
///
/// # Errors
/// Returns a description when the body is not a `ListBucketResult` or
/// says it is truncated without giving a continuation token.
pub fn parse(xml: &str) -> Result<Page, String> {
    let root = element(xml, "ListBucketResult").ok_or("not a ListBucketResult")?;
    let url_encoded = element(root, "EncodingType").is_some_and(|e| e.trim() == "url");
    let truncated = element(root, "IsTruncated").is_some_and(|t| t.trim() == "true");
    let next = element(root, "NextContinuationToken").map(unescape);
    if truncated && next.is_none() {
        return Err("truncated listing without a continuation token".into());
    }
    let objects = elements(root, "Contents")
        .map(|c| {
            let key = unescape(element(c, "Key")?);
            let key = if url_encoded { url_decode(&key)? } else { key };
            let size = element(c, "Size")
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(0);
            let modified = element(c, "LastModified")
                .and_then(parse_timestamp)
                .unwrap_or(UNIX_EPOCH);
            Some(Object {
                key,
                size,
                modified,
            })
        })
        .collect();
    Ok(Page {
        objects,
        next: next.filter(|_| truncated),
    })
}

/// The unescaped, trimmed text of the first `<tag>` in `xml`, for the
/// other small XML answers the adapter reads (STS).
pub fn element_text(xml: &str, tag: &str) -> Option<String> {
    element(xml, tag).map(|text| unescape(text.trim()))
}

/// The text of the first `<tag>` in `xml`; `<tag/>` reads as empty.
fn element<'a>(xml: &'a str, tag: &'a str) -> Option<&'a str> {
    elements(xml, tag).next()
}

/// The text of every `<tag>` in `xml`, in order.
fn elements<'a>(xml: &'a str, tag: &'a str) -> impl Iterator<Item = &'a str> + 'a {
    let mut rest = xml;
    std::iter::from_fn(move || {
        loop {
            let start = rest.find('<')?;
            let after = rest.get(start + 1..)?;
            let Some(after_name) = after.strip_prefix(tag) else {
                rest = after;
                continue;
            };
            let close = after_name.find('>')?;
            let attrs = after_name.get(..close)?;
            // `<tagX>` is a different element; `<tag attr="">` and `<tag/>` match.
            if !(attrs.is_empty() || attrs.starts_with(char::is_whitespace) || attrs == "/") {
                rest = after;
                continue;
            }
            let body = after_name.get(close + 1..)?;
            if attrs.ends_with('/') {
                rest = body;
                return Some("");
            }
            let end_tag = format!("</{tag}>");
            let end = body.find(&end_tag)?;
            rest = body.get(end + end_tag.len()..)?;
            return body.get(..end);
        }
    })
}

/// Replaces the predefined XML entities and character references.
fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(rest.get(..amp).unwrap_or_default());
        let tail = rest.get(amp..).unwrap_or_default();
        let entity = tail.find(';').and_then(|semi| {
            let name = tail.get(1..semi)?;
            let c = match name {
                "amp" => '&',
                "lt" => '<',
                "gt" => '>',
                "quot" => '"',
                "apos" => '\'',
                _ => {
                    let code = match name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
                        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                        None => name.strip_prefix('#')?.parse().ok()?,
                    };
                    char::from_u32(code)?
                }
            };
            Some((c, semi))
        });
        if let Some((c, semi)) = entity {
            out.push(c);
            rest = tail.get(semi + 1..).unwrap_or_default();
        } else {
            out.push('&');
            rest = tail.get(1..).unwrap_or_default();
        }
    }
    out.push_str(rest);
    out
}

/// Decodes `%XY` escapes and `+` (a space, as S3 writes it with
/// `encoding-type=url`). `None` when the result is not UTF-8.
pub fn url_decode(text: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(text.len());
    let mut it = text.bytes();
    while let Some(b) = it.next() {
        match b {
            b'+' => bytes.push(b' '),
            b'%' => {
                let hi = char::from(it.next()?).to_digit(16)?;
                let lo = char::from(it.next()?).to_digit(16)?;
                bytes.push(u8::try_from(hi * 16 + lo).ok()?);
            }
            _ => bytes.push(b),
        }
    }
    String::from_utf8(bytes).ok()
}

/// Parses an ISO 8601 UTC timestamp such as `2009-10-12T17:50:30.000Z`.
pub fn parse_timestamp(text: &str) -> Option<SystemTime> {
    let text = text.trim().strip_suffix('Z')?;
    let (date, time) = text.split_once('T')?;
    let mut date = date.splitn(3, '-');
    let year: i64 = date.next()?.parse().ok()?;
    let month: u32 = date.next()?.parse().ok()?;
    let day: u32 = date.next()?.parse().ok()?;
    let mut time = time.splitn(3, ':');
    let hour: u64 = time.next()?.parse().ok()?;
    let minute: u64 = time.next()?.parse().ok()?;
    let sec_field = time.next()?;
    let (sec, frac) = sec_field.split_once('.').unwrap_or((sec_field, ""));
    let sec: u64 = sec.parse().ok()?;
    let millis: u64 = format!("{frac:0<3}").get(..3)?.parse().ok()?;
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || sec > 60
    {
        return None;
    }
    let days = u64::try_from(days_from_civil(year, month, day)).ok()?;
    let secs = days * 86_400 + hour * 3600 + minute * 60 + sec;
    Some(UNIX_EPOCH + Duration::from_secs(secs) + Duration::from_millis(millis))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
  <Name>bucket</Name><Prefix>notes/</Prefix><KeyCount>3</KeyCount><MaxKeys>1000</MaxKeys>
  <EncodingType>url</EncodingType>
  <IsTruncated>true</IsTruncated>
  <NextContinuationToken>1ueGcxLPRx1Tr/XYExHnhbYLgveDs2J/wm36Hy4vbOwM=&amp;x</NextContinuationToken>
  <Contents>
    <Key>notes/a+b%2Bc.md</Key>
    <LastModified>2009-10-12T17:50:30.000Z</LastModified>
    <ETag>&quot;fba9dede5f27731c9771645a39863328&quot;</ETag>
    <Size>434234</Size>
    <StorageClass>STANDARD</StorageClass>
  </Contents>
  <Contents>
    <Key>notes/%C3%BC/%E2%80%93.md</Key>
    <LastModified>2026-10-29T08:01:02Z</LastModified>
    <Size>0</Size>
  </Contents>
  <Contents><Key>notes/%FF</Key><Size>1</Size></Contents>
</ListBucketResult>"#;

    #[test]
    fn parses_a_truncated_url_encoded_page() -> Result<(), String> {
        let page = parse(PAGE)?;
        assert_eq!(
            page.next.as_deref(),
            Some("1ueGcxLPRx1Tr/XYExHnhbYLgveDs2J/wm36Hy4vbOwM=&x")
        );
        let [Some(first), Some(second), None] = page.objects.as_slice() else {
            return Err(format!("{:?}", page.objects));
        };
        assert_eq!(first.key, "notes/a b+c.md");
        assert_eq!(first.size, 434_234);
        assert_eq!(
            first.modified,
            UNIX_EPOCH + Duration::from_secs(1_255_369_830)
        );
        assert_eq!(second.key, "notes/ü/–.md");
        assert_eq!(second.size, 0);
        assert_eq!(
            second.modified,
            UNIX_EPOCH + Duration::from_secs(1_793_260_862)
        );
        Ok(())
    }

    #[test]
    fn plain_keys_are_only_xml_unescaped() -> Result<(), String> {
        let xml = "<ListBucketResult><IsTruncated>false</IsTruncated>\
                   <Contents><Key>a+b &amp; c&#x2F;&#47;.md</Key><Size>2</Size></Contents>\
                   <NextContinuationToken>ignored</NextContinuationToken>\
                   </ListBucketResult>";
        let page = parse(xml)?;
        assert_eq!(page.next, None);
        assert_eq!(
            page.objects
                .first()
                .and_then(Option::as_ref)
                .map(|o| o.key.as_str()),
            Some("a+b & c//.md")
        );
        Ok(())
    }

    #[test]
    fn empty_and_malformed_bodies() {
        let empty = parse(
            "<ListBucketResult><Prefix/><KeyCount>0</KeyCount><IsTruncated>false</IsTruncated></ListBucketResult>",
        );
        assert_eq!(
            empty,
            Ok(Page {
                objects: vec![],
                next: None
            })
        );
        assert!(parse("<Error><Code>NoSuchBucket</Code></Error>").is_err());
        assert!(
            parse("<ListBucketResult><IsTruncated>true</IsTruncated></ListBucketResult>").is_err()
        );
    }

    #[test]
    fn similar_tag_names_do_not_match() {
        assert_eq!(
            element("<KeyCount>1</KeyCount><Key>k</Key>", "Key"),
            Some("k")
        );
        assert_eq!(element("<Key a=\"1\">k</Key>", "Key"), Some("k"));
    }

    #[test]
    fn timestamps() {
        assert_eq!(
            parse_timestamp("1970-01-01T00:00:01Z"),
            Some(UNIX_EPOCH + Duration::from_secs(1))
        );
        assert_eq!(
            parse_timestamp("1970-01-01T00:00:01.5Z"),
            Some(UNIX_EPOCH + Duration::from_millis(1500))
        );
        assert_eq!(parse_timestamp("1970-13-01T00:00:00Z"), None);
        assert_eq!(parse_timestamp("yesterday"), None);
    }
}
