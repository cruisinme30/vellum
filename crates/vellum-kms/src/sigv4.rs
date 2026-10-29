// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! AWS Signature Version 4, just enough for JSON POSTs to AWS KMS.

use std::fmt::Write as _;
use std::time::{SystemTime, UNIX_EPOCH};

use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};
use vellum_crypto::CryptoError;

const ALGORITHM: &str = "AWS4-HMAC-SHA256";

/// The parts of a request that the signature covers.
pub struct Request<'a> {
    pub method: &'a str,
    pub path: &'a str,
    /// Already canonical: sorted, URI-encoded `name=value` pairs joined by `&`.
    pub query: &'a str,
    /// Lower-case names with their values; `host` and `x-amz-date` included.
    pub headers: &'a [(&'a str, &'a str)],
    pub body: &'a [u8],
}

/// Where and as whom a request is signed.
pub struct Scope<'a> {
    /// `YYYYMMDDTHHMMSSZ`, also sent as `x-amz-date`.
    pub amz_date: &'a str,
    pub region: &'a str,
    pub service: &'a str,
    pub access_key_id: &'a str,
    pub secret_access_key: &'a str,
}

/// The `Authorization` header value for `request`.
pub fn authorization(request: &Request<'_>, scope: &Scope<'_>) -> Result<String, CryptoError> {
    let mut headers: Vec<(String, String)> = request
        .headers
        .iter()
        .map(|(name, value)| (name.to_ascii_lowercase(), collapse_spaces(value)))
        .collect();
    headers.sort();
    let signed_headers = headers
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>()
        .join(";");
    let mut canonical = format!("{}\n{}\n{}\n", request.method, request.path, request.query);
    for (name, value) in &headers {
        let _ = writeln!(canonical, "{name}:{value}");
    }
    let _ = write!(
        canonical,
        "\n{signed_headers}\n{}",
        hex(&Sha256::digest(request.body))
    );

    let date = scope
        .amz_date
        .get(..8)
        .ok_or(CryptoError::Malformed("x-amz-date"))?;
    let credential_scope = format!("{date}/{}/{}/aws4_request", scope.region, scope.service);
    let string_to_sign = format!(
        "{ALGORITHM}\n{}\n{credential_scope}\n{}",
        scope.amz_date,
        hex(&Sha256::digest(canonical.as_bytes()))
    );

    let secret = zeroize::Zeroizing::new(format!("AWS4{}", scope.secret_access_key));
    let k_date = hmac(secret.as_bytes(), date.as_bytes())?;
    let k_region = hmac(&k_date, scope.region.as_bytes())?;
    let k_service = hmac(&k_region, scope.service.as_bytes())?;
    let k_signing = hmac(&k_service, b"aws4_request")?;
    let signature = hex(&hmac(&k_signing, string_to_sign.as_bytes())?);

    Ok(format!(
        "{ALGORITHM} Credential={}/{credential_scope}, SignedHeaders={signed_headers}, Signature={signature}",
        scope.access_key_id
    ))
}

/// The current time as `YYYYMMDDTHHMMSSZ`.
pub fn amz_date_now() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    amz_date(secs)
}

/// Formats Unix seconds as `YYYYMMDDTHHMMSSZ` (UTC, proleptic Gregorian).
pub fn amz_date(unix_secs: u64) -> String {
    let days = unix_secs / 86_400;
    let rem = unix_secs % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}{month:02}{day:02}T{:02}{:02}{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Days since 1970-01-01 to a calendar date (Howard Hinnant's algorithm,
/// restricted to dates on or after the epoch).
const fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };
    (year, month, day)
}

fn hmac(key: &[u8], data: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(key)
        .map_err(|_| CryptoError::Provider("aws-kms: cannot key HMAC".into()))?;
    mac.update(data);
    Ok(mac.finalize().into_bytes().to_vec())
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

fn collapse_spaces(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The worked example from the AWS `SigV4` documentation (IAM `ListUsers`).
    #[test]
    fn matches_the_documented_example() -> Result<(), CryptoError> {
        let request = Request {
            method: "GET",
            path: "/",
            query: "Action=ListUsers&Version=2010-05-08",
            headers: &[
                (
                    "content-type",
                    "application/x-www-form-urlencoded; charset=utf-8",
                ),
                ("host", "iam.amazonaws.com"),
                ("x-amz-date", "20150830T123600Z"),
            ],
            body: b"",
        };
        let scope = Scope {
            amz_date: "20150830T123600Z",
            region: "us-east-1",
            service: "iam",
            access_key_id: "AKIDEXAMPLE",
            secret_access_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
        };
        assert_eq!(
            authorization(&request, &scope)?,
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/iam/aws4_request, \
             SignedHeaders=content-type;host;x-amz-date, \
             Signature=5d672d79c15b13162d9279b0855cfba6789a8edb4c82c400e06b5924a6f2b5d7"
        );
        Ok(())
    }

    #[test]
    fn formats_dates() {
        assert_eq!(amz_date(0), "19700101T000000Z");
        assert_eq!(amz_date(1_440_938_160), "20150830T123600Z");
        assert_eq!(amz_date(951_782_400), "20000229T000000Z");
        assert_eq!(amz_date(1_861_920_000), "20290101T000000Z");
    }
}
