// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The S3 adapter passes the storage conformance suite against `LocalStack`
//! (started with the docker CLI), bare, wrapped in the metrics decorator,
//! and under a key prefix.
//!
//! `LocalStack` runs with signature validation on, so every request also
//! proves the hand-rolled `SigV4` signing. Without docker the tests fail
//! when `CI` is set and are skipped (with a note on stderr) otherwise.

use std::{
    process::Command,
    time::{Duration, Instant},
};

use bytes::Bytes;
use futures::{StreamExt, TryStreamExt};
use vellum_store::{
    BlobPath, BlobStore, Metered, Precondition, conformance,
    remote::{
        Secret,
        s3::{Addressing, S3Config, S3Credentials, S3Store},
    },
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const IMAGE: &str = "localstack/localstack:4";

/// A `LocalStack` container, removed on drop.
struct LocalStack {
    id: String,
    endpoint: String,
}

impl Drop for LocalStack {
    fn drop(&mut self) {
        let _ = Command::new("docker").args(["rm", "-f", &self.id]).output();
    }
}

fn docker(args: &[&str]) -> Result<String, String> {
    let out = Command::new("docker")
        .args(args)
        .output()
        .map_err(|e| format!("docker {}: {e}", args.join(" ")))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    } else {
        Err(format!(
            "docker {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// Starts `LocalStack` with a bucket named `bucket`, or returns `None` when
/// docker is unavailable outside CI.
async fn localstack(bucket: &str) -> Result<Option<LocalStack>, Box<dyn std::error::Error>> {
    if let Err(e) = docker(&["info", "--format", "{{.ServerVersion}}"]) {
        if std::env::var_os("CI").is_some() {
            return Err(format!("docker is required in CI: {e}").into());
        }
        eprintln!("note: skipping the S3 conformance test, docker is unavailable: {e}");
        return Ok(None);
    }
    let id = docker(&[
        "run",
        "-d",
        "--rm",
        "-p",
        "127.0.0.1::4566",
        "-e",
        "S3_SKIP_SIGNATURE_VALIDATION=0",
        IMAGE,
    ])?;
    let mut stack = LocalStack {
        id,
        endpoint: String::new(),
    };
    let port = docker(&["port", &stack.id, "4566/tcp"])?;
    let port = port.lines().next().unwrap_or_default();
    stack.endpoint = format!("http://{port}");
    wait_for_s3(&stack.endpoint).await?;
    let _ = docker(&[
        "exec",
        &stack.id,
        "awslocal",
        "s3api",
        "create-bucket",
        "--bucket",
        bucket,
    ])?;
    Ok(Some(stack))
}

async fn wait_for_s3(endpoint: &str) -> Result<(), Box<dyn std::error::Error>> {
    let health = format!("{endpoint}/_localstack/health");
    let deadline = Instant::now() + Duration::from_mins(2);
    loop {
        if let Ok(response) = reqwest::get(&health).await
            && let Ok(body) = response.json::<serde_json::Value>().await
            && matches!(
                body.pointer("/services/s3").and_then(|v| v.as_str()),
                Some("available" | "running")
            )
        {
            return Ok(());
        }
        if Instant::now() > deadline {
            return Err("LocalStack S3 did not become available within 120 s".into());
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

fn store(
    stack: &LocalStack,
    bucket: &str,
    prefix: Option<&str>,
) -> Result<S3Store, vellum_store::StoreError> {
    S3Store::new(S3Config {
        endpoint: stack.endpoint.clone(),
        region: "us-east-1".into(),
        bucket: bucket.into(),
        prefix: prefix.map(Into::into),
        addressing: Addressing::Path,
        credentials: S3Credentials {
            access_key_id: "test".into(),
            secret_access_key: Secret::new("test"),
            session_token: None,
        }
        .into(),
    })
}

/// One container for every scenario keeps the test fast (nextest runs each
/// test in its own process, so they cannot share one); each scenario
/// gets its own bucket or prefix because the suite needs `conformance/`
/// to start empty.
#[tokio::test(flavor = "multi_thread")]
async fn s3_store_conforms() -> TestResult {
    let Some(stack) = localstack("bare").await? else {
        return Ok(());
    };
    for bucket in ["metered", "prefixed", "odd", "pages"] {
        let _ = docker(&[
            "exec",
            &stack.id,
            "awslocal",
            "s3api",
            "create-bucket",
            "--bucket",
            bucket,
        ])?;
    }

    conformance::run_all(&store(&stack, "bare", None)?).await?;
    conformance::run_all(&Metered::new(store(&stack, "metered", None)?)).await?;

    // Under a prefix, and with a neighbour prefix that must stay invisible.
    let outside = store(&stack, "prefixed", Some("other"))?;
    let _ = outside
        .put(
            &BlobPath::new("conformance/list_is_recursive_and_scoped/d/stray.md")?,
            Bytes::from_static(b"x"),
            Precondition::None,
        )
        .await?;
    let prefixed = store(&stack, "prefixed", Some("team/one"))?;
    conformance::run_all(&prefixed).await?;
    conformance::run_all(&Metered::new(store(&stack, "prefixed", Some("team/two"))?)).await?;
    let mine: Vec<_> = prefixed.list(None).try_collect().await?;
    assert!(
        mine.iter()
            .all(|e| e.path.as_str().starts_with("conformance/")),
        "{mine:?}"
    );

    odd_keys_round_trip_through_listings(&stack).await?;
    listings_span_pages(&stack).await
}

/// Keys that need escaping survive a put, a listing and a get.
async fn odd_keys_round_trip_through_listings(stack: &LocalStack) -> TestResult {
    let store = store(stack, "odd", Some("p"))?;
    let names = [
        "a b+c.md",
        "Notes – ü/x&y<z>.md",
        "%41 'q' (1) [2] {3} #4 $5 @6 =7 ;8 ,9 ~!*.md",
    ];
    for name in names {
        let _ = store
            .put(
                &BlobPath::new(name)?,
                Bytes::from(name),
                Precondition::IfAbsent,
            )
            .await?;
    }
    let mut listed: Vec<String> = store
        .list(None)
        .map_ok(|e| e.path.to_string())
        .try_collect()
        .await?;
    listed.sort();
    let mut want: Vec<String> = names.iter().map(|n| (*n).to_owned()).collect();
    want.sort();
    assert_eq!(listed, want);
    for name in names {
        let got = store.get(&BlobPath::new(name)?).await?;
        assert_eq!(got.bytes.as_ref(), name.as_bytes());
    }
    Ok(())
}

/// Listings follow continuation tokens past the 1000-key page size.
async fn listings_span_pages(stack: &LocalStack) -> TestResult {
    let store = store(stack, "pages", None)?;
    let writes = (0..1100).map(|i| {
        let store = store.clone();
        async move {
            let path = BlobPath::new(format!("many/{i:04}.md"))?;
            store.put(&path, Bytes::new(), Precondition::None).await
        }
    });
    let _ = futures::stream::iter(writes)
        .buffer_unordered(32)
        .try_collect::<Vec<_>>()
        .await?;
    let listed: Vec<_> = store
        .list(Some(&BlobPath::new("many")?))
        .try_collect()
        .await?;
    assert_eq!(listed.len(), 1100);
    Ok(())
}
