// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! AWS Signature Version 4, header-based, as S3 documents it.
//!
//! Hand-rolled on `hmac` and `sha2` so the adapter needs no AWS SDK. The
//! tests check it against the examples AWS publishes.

use std::{
    fmt::Write as _,
    time::{SystemTime, UNIX_EPOCH},
};

use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};

use crate::error::{StoreError, StoreResult};

/// SHA-256 of an empty payload, sent with every bodiless request.
pub const EMPTY_SHA256: &str =
    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

/// The request parts that `SigV4` signs.
pub struct Canonical<'a> {
    /// HTTP method, upper case.
    pub method: &'a str,
    /// The path exactly as sent, already URI-encoded.
    pub uri: &'a str,
    /// The query string exactly as sent, from [`canonical_query`].
    pub query: &'a str,
    /// Header names in lower case, with their values. Must include `host`
    /// and `x-amz-date`.
    pub headers: &'a [(&'a str, &'a str)],
    /// Hex SHA-256 of the body.
    pub payload_hash: &'a str,
}

/// Who signs, and for which region and service.
pub struct Key<'a> {
    /// Access key id.
    pub access_key_id: &'a str,
    /// Secret access key.
    pub secret: &'a str,
    /// Region, such as `us-east-1`.
    pub region: &'a str,
    /// Service, `s3` for this adapter.
    pub service: &'a str,
}

/// The `x-amz-date` timestamp and the date of the credential scope.
pub struct AmzDate {
    /// `YYYYMMDD`.
    pub date: String,
    /// `YYYYMMDDTHHMMSSZ`.
    pub stamp: String,
}

impl AmzDate {
    /// The timestamp for `at`, in UTC.
    pub fn at(at: SystemTime) -> Self {
        let secs = at
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs().cast_signed());
        let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
        let (y, m, d) = civil_from_days(days);
        let date = format!("{y:04}{m:02}{d:02}");
        let stamp = format!(
            "{date}T{:02}{:02}{:02}Z",
            rem / 3600,
            rem % 3600 / 60,
            rem % 60
        );
        Self { date, stamp }
    }
}

/// The `Authorization` header value for `req`.
///
/// # Errors
/// Returns [`StoreError::Backend`] if the MAC cannot be keyed (HMAC takes
/// keys of any length, so this does not happen in practice).
pub fn authorization(
    key: &Key<'_>,
    req: &Canonical<'_>,
    when: &AmzDate,
) -> StoreResult<String> {
    let (canonical, signed_headers) = canonical_request(req);
    let scope =
        format!("{}/{}/{}/aws4_request", when.date, key.region, key.service);
    let to_sign = format!(
        "AWS4-HMAC-SHA256\n{}\n{scope}\n{}",
        when.stamp,
        sha256_hex(canonical.as_bytes())
    );
    let signing = signing_key(key, &when.date)?;
    let signature = hex::encode(hmac(&signing, to_sign.as_bytes())?);
    Ok(format!(
        "AWS4-HMAC-SHA256 Credential={}/{scope}, \
            SignedHeaders={signed_headers}, Signature={signature}",
        key.access_key_id
    ))
}

/// The canonical request and the `SignedHeaders` list.
fn canonical_request(req: &Canonical<'_>) -> (String, String) {
    let mut headers: Vec<(String, String)> = req
        .headers
        .iter()
        .map(|(n, v)| (n.to_ascii_lowercase(), collapse_spaces(v)))
        .collect();
    headers.sort();
    let mut canonical_headers = String::new();
    for (name, value) in &headers {
        let _ = writeln!(canonical_headers, "{name}:{value}");
    }
    let signed: Vec<&str> = headers.iter().map(|(n, _)| n.as_str()).collect();
    let signed = signed.join(";");
    let canonical = format!(
        "{}\n{}\n{}\n{canonical_headers}\n{signed}\n{}",
        req.method, req.uri, req.query, req.payload_hash
    );
    (canonical, signed)
}

fn collapse_spaces(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Derives the per-day signing key.
fn signing_key(key: &Key<'_>, date: &str) -> StoreResult<Vec<u8>> {
    let k_date =
        hmac(format!("AWS4{}", key.secret).as_bytes(), date.as_bytes())?;
    let k_region = hmac(&k_date, key.region.as_bytes())?;
    let k_service = hmac(&k_region, key.service.as_bytes())?;
    hmac(&k_service, b"aws4_request")
}

fn hmac(key: &[u8], data: &[u8]) -> StoreResult<Vec<u8>> {
    let mut mac =
        <Hmac<Sha256> as KeyInit>::new_from_slice(key).map_err(|e| {
            StoreError::Backend(format!("s3: cannot key HMAC: {e}"))
        })?;
    mac.update(data);
    Ok(mac.finalize().into_bytes().to_vec())
}

/// Hex SHA-256 of `bytes`, for `x-amz-content-sha256`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// `SigV4` `UriEncode`: every byte but `A-Z a-z 0-9 - _ . ~` becomes `%XY`
/// (upper-case hex), and `/` is kept only when `keep_slash` is set.
pub fn uri_encode(raw: &str, keep_slash: bool) -> String {
    let mut out = String::with_capacity(raw.len());
    for &b in raw.as_bytes() {
        if b.is_ascii_alphanumeric()
            || matches!(b, b'-' | b'_' | b'.' | b'~')
            || (keep_slash && b == b'/')
        {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// Encodes and sorts query parameters into the canonical query string,
/// which is also what goes on the wire.
pub fn canonical_query(params: &[(&str, &str)]) -> String {
    let mut encoded: Vec<(String, String)> = params
        .iter()
        .map(|(k, v)| (uri_encode(k, false), uri_encode(v, false)))
        .collect();
    encoded.sort();
    let pairs: Vec<String> = encoded
        .into_iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect();
    pairs.join("&")
}

/// Days since 1970-01-01 to a proleptic Gregorian `(year, month, day)`
/// (Howard Hinnant's `civil_from_days`).
pub const fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "m is in 1..=12 and d in 1..=31"
    )]
    (y, m as u32, d as u32)
}

/// A proleptic Gregorian date to days since 1970-01-01 (Hinnant's
/// `days_from_civil`).
pub const fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let m = month as i64;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    const S3_KEY: Key<'static> = Key {
        access_key_id: "AKIAIOSFODNN7EXAMPLE",
        secret: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        region: "us-east-1",
        service: "s3",
    };

    fn may_24_2013() -> AmzDate {
        AmzDate {
            date: "20130524".into(),
            stamp: "20130524T000000Z".into(),
        }
    }

    fn signature(auth: &str) -> &str {
        auth.rsplit("Signature=").next().unwrap_or_default()
    }

    /// "Example: GET Object" from the S3 API reference, "Signature
    /// Calculations for the Authorization Header".
    #[test]
    fn aws_example_get_object() -> StoreResult<()> {
        let headers = [
            ("host", "examplebucket.s3.amazonaws.com"),
            ("range", "bytes=0-9"),
            ("x-amz-content-sha256", EMPTY_SHA256),
            ("x-amz-date", "20130524T000000Z"),
        ];
        let req = Canonical {
            method: "GET",
            uri: "/test.txt",
            query: "",
            headers: &headers,
            payload_hash: EMPTY_SHA256,
        };
        let (canonical, _) = canonical_request(&req);
        assert_eq!(
            sha256_hex(canonical.as_bytes()),
            "7344ae5b7ee6c3e7e6b0fe0640412a37625d1fbfff95c48bbb2dc43964946972"
        );
        let auth = authorization(&S3_KEY, &req, &may_24_2013())?;
        assert_eq!(
            auth,
            "AWS4-HMAC-SHA256 \
             Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request\
             , SignedHeaders=host;range;x-amz-content-sha256;x-amz-date, \
             Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6\
             036bdb41"
        );
        Ok(())
    }

    /// "Example: PUT Object": a `$` in the key and a signed body.
    #[test]
    fn aws_example_put_object() -> StoreResult<()> {
        let body_hash = sha256_hex(b"Welcome to Amazon S3.");
        assert_eq!(
            body_hash,
            "44ce7dd67c959e0d3524ffac1771dfbba87d2b6b4b4e99e42034a8b803f8b072"
        );
        let uri = format!("/{}", uri_encode("test$file.text", true));
        assert_eq!(uri, "/test%24file.text");
        let headers = [
            ("Date", "Fri, 24 May 2013 00:00:00 GMT"),
            ("Host", "examplebucket.s3.amazonaws.com"),
            ("x-amz-content-sha256", body_hash.as_str()),
            ("x-amz-date", "20130524T000000Z"),
            ("x-amz-storage-class", "REDUCED_REDUNDANCY"),
        ];
        let req = Canonical {
            method: "PUT",
            uri: &uri,
            query: "",
            headers: &headers,
            payload_hash: &body_hash,
        };
        let auth = authorization(&S3_KEY, &req, &may_24_2013())?;
        assert_eq!(
            signature(&auth),
            "98ad721746da40c64f1a55b78f14c238d841ea1380cd77a1b5971af0ece108bd"
        );
        Ok(())
    }

    /// "Example: GET Bucket Lifecycle": a query parameter without a value.
    #[test]
    fn aws_example_get_bucket_lifecycle() -> StoreResult<()> {
        let query = canonical_query(&[("lifecycle", "")]);
        assert_eq!(query, "lifecycle=");
        let headers = [
            ("host", "examplebucket.s3.amazonaws.com"),
            ("x-amz-content-sha256", EMPTY_SHA256),
            ("x-amz-date", "20130524T000000Z"),
        ];
        let req = Canonical {
            method: "GET",
            uri: "/",
            query: &query,
            headers: &headers,
            payload_hash: EMPTY_SHA256,
        };
        let auth = authorization(&S3_KEY, &req, &may_24_2013())?;
        assert_eq!(
            signature(&auth),
            "fea454ca298b7da1c68078a5d1bdbfbbe0d65c699e0f91ac7a200a0136783543"
        );
        Ok(())
    }

    /// "Example: Get Bucket (List Objects)": query parameters are sorted.
    #[test]
    fn aws_example_list_objects() -> StoreResult<()> {
        let query = canonical_query(&[("prefix", "J"), ("max-keys", "2")]);
        assert_eq!(query, "max-keys=2&prefix=J");
        let headers = [
            ("host", "examplebucket.s3.amazonaws.com"),
            ("x-amz-content-sha256", EMPTY_SHA256),
            ("x-amz-date", "20130524T000000Z"),
        ];
        let req = Canonical {
            method: "GET",
            uri: "/",
            query: &query,
            headers: &headers,
            payload_hash: EMPTY_SHA256,
        };
        let auth = authorization(&S3_KEY, &req, &may_24_2013())?;
        assert_eq!(
            signature(&auth),
            "34b48302e7b5fa45bde8084f4b7868a86f0a534bc59db6670ed5711ef69dc6f7"
        );
        Ok(())
    }

    /// `get-vanilla` from the AWS `SigV4` test suite (a non-S3 service).
    #[test]
    fn aws_test_suite_get_vanilla() -> StoreResult<()> {
        let key = Key {
            access_key_id: "AKIDEXAMPLE",
            secret: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
            region: "us-east-1",
            service: "service",
        };
        let headers = [
            ("Host", "example.amazonaws.com"),
            ("X-Amz-Date", "20150830T123600Z"),
        ];
        let req = Canonical {
            method: "GET",
            uri: "/",
            query: "",
            headers: &headers,
            payload_hash: EMPTY_SHA256,
        };
        let when = AmzDate::at(UNIX_EPOCH + Duration::from_mins(24_015_636));
        assert_eq!(when.stamp, "20150830T123600Z");
        let auth = authorization(&key, &req, &when)?;
        assert_eq!(
            auth,
            "AWS4-HMAC-SHA256 \
             Credential=AKIDEXAMPLE/20150830/us-east-1/service/aws4_request, \
             SignedHeaders=host;x-amz-date, \
             Signature=5fa00fa31553b73ebf1942676e86291e8372ff2a2260956d9b8aae1d\
             763fbf31"
        );
        Ok(())
    }

    /// The signing-key derivation example from the IAM user guide.
    #[test]
    fn aws_example_signing_key() -> StoreResult<()> {
        let key = Key {
            access_key_id: "",
            secret: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
            region: "us-east-1",
            service: "iam",
        };
        assert_eq!(
            hex::encode(signing_key(&key, "20120215")?),
            "f4780e2d9f65fa895f9c67b32ce1baf0b0d8a43505a000a1a9e090d414db404d"
        );
        Ok(())
    }

    #[test]
    fn encodes_each_segment_and_keeps_slashes() {
        assert_eq!(
            uri_encode("a b/Notes – ü+x~y_z.md", true),
            "a%20b/Notes%20%E2%80%93%20%C3%BC%2Bx~y_z.md"
        );
        assert_eq!(uri_encode("a/b", false), "a%2Fb");
        assert_eq!(uri_encode("*'()!", true), "%2A%27%28%29%21");
    }

    #[test]
    fn canonical_headers_are_trimmed_lowercased_and_sorted() {
        let headers = [("X-B", "  two   words "), ("x-a", "1")];
        let req = Canonical {
            method: "GET",
            uri: "/",
            query: "",
            headers: &headers,
            payload_hash: EMPTY_SHA256,
        };
        let (canonical, signed) = canonical_request(&req);
        assert_eq!(signed, "x-a;x-b");
        assert!(
            canonical.contains("\nx-a:1\nx-b:two words\n\n"),
            "{canonical}"
        );
    }

    #[test]
    fn civil_dates_round_trip() {
        for (days, ymd) in [
            (0, (1970, 1, 1)),
            (-1, (1969, 12, 31)),
            (11_016, (2000, 2, 29)),
            (20_755, (2026, 10, 29)),
        ] {
            assert_eq!(civil_from_days(days), ymd);
            assert_eq!(days_from_civil(ymd.0, ymd.1, ymd.2), days);
        }
    }
}
