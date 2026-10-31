// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The Git backend passes the conformance suite, commits per batch, and
//! syncs through the three-way merge: clean merges are taken, conflicts
//! keep the remote version and copy out the local one, and an edit beats
//! a delete.

use std::{path::Path, process::Command};

use bytes::Bytes;
use futures::TryStreamExt;
use vellum_store::{
    BlobPath, BlobStore, Precondition, Writer, conformance,
    git::{GitConfig, GitStore},
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn config(name: &str) -> GitConfig {
    GitConfig {
        remote: "origin".into(),
        branch: "main".into(),
        author_name: name.into(),
        author_email: format!("{name}@northstar.dev"),
    }
}

fn p(s: &str) -> Result<BlobPath, vellum_store::StoreError> {
    BlobPath::new(s)
}

fn reidentify(bytes: &[u8]) -> Bytes {
    std::str::from_utf8(bytes).map_or_else(
        |_| Bytes::copy_from_slice(bytes),
        |text| Bytes::from(text.replace("id: A", "id: COPY")),
    )
}

fn writer() -> Writer<'static> {
    Writer {
        user: "Ana",
        date: "2026-10-29",
        reidentify: &reidentify,
    }
}

fn git(
    dir: &Path,
    args: &[&str],
) -> Result<String, Box<dyn std::error::Error>> {
    let out = Command::new("git").arg("-C").arg(dir).args(args).output()?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).into_owned().into());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// A bare remote and two stores cloned from it, both at the same commit.
async fn pair()
-> Result<(tempfile::TempDir, GitStore, GitStore), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let remote = tmp.path().join("remote.git");
    let _ = git(
        tmp.path(),
        &[
            "init",
            "--quiet",
            "--bare",
            "--initial-branch=main",
            "remote.git",
        ],
    )?;
    let url = remote.display().to_string();
    let a = GitStore::open(tmp.path().join("a"), config("ana")).await?;
    let _ = git(a.dir(), &["remote", "add", "origin", &url])?;
    let _ = a
        .put(&p("Plan.md")?, doc("one\ntwo\nthree\n"), Precondition::None)
        .await?;
    let _ = a
        .put(
            &p("logo.png")?,
            Bytes::from_static(b"\x89PNG"),
            Precondition::None,
        )
        .await?;
    let first = a.sync("seed", &writer()).await?;
    assert!(first.pushed, "{first:?}");
    let b = GitStore::open(tmp.path().join("b"), config("ben")).await?;
    let _ = git(b.dir(), &["remote", "add", "origin", &url])?;
    let pulled = b.sync("nothing yet", &writer()).await?;
    assert!(
        pulled.updated.iter().any(|u| u.as_str() == "Plan.md"),
        "{pulled:?}"
    );
    Ok((tmp, a, b))
}

fn doc(body: &str) -> Bytes {
    Bytes::from(format!("---\nid: A\n---\n{body}"))
}

async fn read(
    store: &GitStore,
    path: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let blob = store.get(&p(path)?).await?;
    Ok(String::from_utf8(blob.bytes.to_vec())?)
}

#[tokio::test]
async fn passes_conformance_and_hides_the_git_dir() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let store = GitStore::open(tmp.path(), config("ana")).await?;
    conformance::run_all(&store).await?;
    let listed: Vec<_> = store.list(None).try_collect().await?;
    assert!(listed.iter().all(|e| !e.path.as_str().starts_with(".git")));
    // The Git directory cannot even be named, in any spelling (#272).
    for git in [
        ".git/HEAD",
        ".git/hooks/x",
        ".GIT/config",
        "sub/.git/config",
    ] {
        assert!(p(git).is_err(), "{git:?} should be refused");
    }
    Ok(())
}

#[tokio::test]
async fn commits_one_batch_and_skips_vellum_state() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let store = GitStore::open(tmp.path(), config("ana")).await?;
    for name in ["a.md", "b.md"] {
        let _ = store
            .put(&p(name)?, Bytes::from_static(b"x"), Precondition::None)
            .await?;
    }
    let _ = store
        .put(
            &p(".vellum/index.sqlite")?,
            Bytes::from_static(b"db"),
            Precondition::None,
        )
        .await?;
    std::fs::write(tmp.path().join("outside.md"), "edited in vim")?;
    let first = store.commit("batch 1").await?;
    assert!(first.is_some());
    assert_eq!(store.commit("nothing changed").await?, None);
    let files = git(tmp.path(), &["ls-files"])?;
    assert_eq!(
        files.lines().collect::<Vec<_>>(),
        ["a.md", "b.md", "outside.md"]
    );
    assert_eq!(
        git(tmp.path(), &["rev-list", "--count", "HEAD"])?.trim(),
        "1"
    );
    Ok(())
}

#[tokio::test]
async fn separate_edits_merge_cleanly() -> TestResult {
    let (_tmp, a, b) = pair().await?;
    let _ = a
        .put(&p("Plan.md")?, doc("ONE\ntwo\nthree\n"), Precondition::None)
        .await?;
    let pushed = a.sync("ana edits", &writer()).await?;
    assert!(pushed.pushed);
    let _ = b
        .put(&p("Plan.md")?, doc("one\ntwo\nTHREE\n"), Precondition::None)
        .await?;
    let synced = b.sync("ben edits", &writer()).await?;
    assert!(synced.pushed && synced.conflicts.is_empty(), "{synced:?}");
    assert_eq!(
        read(&b, "Plan.md").await?,
        "---\nid: A\n---\nONE\ntwo\nTHREE\n"
    );
    let back = a.sync("again", &writer()).await?;
    assert!(back.updated.iter().any(|u| u.as_str() == "Plan.md"));
    assert_eq!(read(&a, "Plan.md").await?, read(&b, "Plan.md").await?);
    Ok(())
}

#[tokio::test]
async fn clashing_edits_keep_the_remote_and_copy_out_ours() -> TestResult {
    let (_tmp, a, b) = pair().await?;
    let _ = a
        .put(&p("Plan.md")?, doc("one\nANA\nthree\n"), Precondition::None)
        .await?;
    let _ = a.sync("ana", &writer()).await?;
    let _ = b
        .put(&p("Plan.md")?, doc("one\nBEN\nthree\n"), Precondition::None)
        .await?;
    let synced = b.sync("ben", &writer()).await?;
    let copy = "Plan (conflict 2026-10-29 Ana).md";
    assert!(
        matches!(
            &synced.conflicts[..],
            [(path, c)] if path.as_str() == "Plan.md" && c.as_str() == copy
        ),
        "{synced:?}"
    );
    assert_eq!(
        read(&b, "Plan.md").await?,
        "---\nid: A\n---\none\nANA\nthree\n"
    );
    assert_eq!(
        read(&b, copy).await?,
        "---\nid: COPY\n---\none\nBEN\nthree\n"
    );
    let _ = a.sync("pull", &writer()).await?;
    assert_eq!(read(&a, copy).await?, read(&b, copy).await?);
    Ok(())
}

#[tokio::test]
async fn an_edit_beats_a_delete_and_binaries_conflict() -> TestResult {
    let (_tmp, a, b) = pair().await?;
    a.delete(&p("Plan.md")?, Precondition::None).await?;
    let _ = a
        .put(
            &p("logo.png")?,
            Bytes::from_static(b"\x89PNG-a"),
            Precondition::None,
        )
        .await?;
    let _ = a.sync("ana deletes", &writer()).await?;
    let _ = b
        .put(
            &p("Plan.md")?,
            doc("one\ntwo\nthree\nfour\n"),
            Precondition::None,
        )
        .await?;
    let _ = b
        .put(
            &p("logo.png")?,
            Bytes::from_static(b"\x89PNG-b"),
            Precondition::None,
        )
        .await?;
    let synced = b.sync("ben edits", &writer()).await?;
    assert!(synced.pushed, "{synced:?}");
    assert_eq!(
        read(&b, "Plan.md").await?,
        "---\nid: A\n---\none\ntwo\nthree\nfour\n"
    );
    let logo = b.get(&p("logo.png")?).await?;
    assert_eq!(logo.bytes.as_ref(), b"\x89PNG-a");
    let copy = b.get(&p("logo (conflict 2026-10-29 Ana).png")?).await?;
    assert_eq!(copy.bytes.as_ref(), b"\x89PNG-b");
    Ok(())
}

#[tokio::test]
async fn sync_without_a_remote_only_commits() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let store = GitStore::open(tmp.path(), config("ana")).await?;
    let _ = store
        .put(&p("a.md")?, Bytes::from_static(b"x"), Precondition::None)
        .await?;
    let synced = store.sync("local only", &writer()).await?;
    assert!(synced.committed.is_some() && !synced.pushed);
    Ok(())
}
