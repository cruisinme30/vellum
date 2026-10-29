---
id: 01M63CFE668Q6YQ8SXHWXWBQCG
title: Writing a storage adapter
vellum: 1
---

# Writing a storage adapter

Content is read and written through the `BlobStore` trait in
`vellum-store`. An adapter implements it for one backend; the local
folder adapter is the reference.

## The contract {#contract}

::: facts

- Keys are paths relative to the drive root, with `/` separators
- Reads return the bytes and a version tag (an ETag, a generation or a commit)
- Conditional writes succeed only if the version still matches, and fail with a conflict error otherwise
- Listing is paginated and may be eventually consistent; the merge layer copes
- Every call creates a tracing span and records latency and bytes

:::

## The conformance suite {#conformance}

`vellum-store` ships a conformance suite that every adapter must pass:
round trips, conditional writes, conflicts, listing, deletes, large
objects and unusual names. Run it from your adapter's tests:

```rust
#[tokio::test]
async fn conforms() -> Result<(), vellum_store::Error> {
    let store = MyStore::connect_for_tests().await?;
    vellum_store::conformance::run(&store).await
}
```

## Eventually consistent backends {#eventual}

Adapters for Google Drive, Dropbox and OneDrive cannot promise
read-after-write. They report it through the adapter's capabilities, and
the content service then confirms every write and merges everything the
watcher sees, rather than trusting arrival order.
