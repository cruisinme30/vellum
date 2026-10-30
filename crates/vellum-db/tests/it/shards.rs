// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Sharded Postgres: a main database and two shards, as three databases in
//! one throwaway server.

use crate::common;

use sqlx::Connection as _;
use sqlx::postgres::PgConnection;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner as _;
use testcontainers_modules::testcontainers::{ContainerAsync, ImageExt as _};
use vellum_db::{Error, IndexConfig, PlainFiles, ShardedIndex, ShardsConfig};
use vellum_kernel::ObjectId;

struct Cluster {
    _container: Option<ContainerAsync<Postgres>>,
    base: String,
    /// Appended to each database name, so tests can share one server.
    suffix: String,
}

impl Cluster {
    /// A throwaway Postgres, or the server at `VELLUM_TEST_POSTGRES`
    /// (`postgres://user:pass@host:port`) when it is set.
    async fn start() -> Result<Self, Box<dyn std::error::Error>> {
        let (container, base) = if let Ok(base) = std::env::var("VELLUM_TEST_POSTGRES") {
            (None, base)
        } else {
            let container = Postgres::default().with_tag("17-alpine").start().await?;
            let port = container.get_host_port_ipv4(5432).await?;
            let base = format!("postgres://postgres:postgres@127.0.0.1:{port}");
            (Some(container), base)
        };
        let suffix = format!("_{}", ObjectId::generate().as_uuid().simple());
        let mut conn = PgConnection::connect(&format!("{base}/postgres")).await?;
        for db in ["main", "eu", "us"] {
            let _ = sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {db}{suffix}")))
                .execute(&mut conn)
                .await?;
        }
        Ok(Self {
            _container: container,
            base,
            suffix,
        })
    }

    fn url(&self, db: &str) -> String {
        format!("{}/{db}{}", self.base, self.suffix)
    }

    fn config(&self) -> ShardsConfig {
        ShardsConfig {
            main: IndexConfig::postgres(self.url("main")),
            shards: vec![
                ("eu".into(), IndexConfig::postgres(self.url("eu"))),
                ("us".into(), IndexConfig::postgres(self.url("us"))),
            ],
        }
    }

    async fn spaces(&self, db: &str) -> sqlx::Result<Vec<String>> {
        let mut conn = PgConnection::connect(&self.url(db)).await?;
        sqlx::query_scalar("SELECT root_path FROM spaces ORDER BY root_path")
            .fetch_all(&mut conn)
            .await
    }

    async fn count(&self, db: &str, sql: &'static str) -> sqlx::Result<i64> {
        let mut conn = PgConnection::connect(&self.url(db)).await?;
        sqlx::query_scalar(sql).fetch_one(&mut conn).await
    }
}

const ALL_SPACES: [&str; 3] = ["drives/shared/engineering", "tasks/ada", "teams/ENG"];

#[tokio::test]
async fn new_spaces_go_home_and_moved_spaces_follow_the_map() {
    let cluster = Cluster::start().await.unwrap();
    let work = tempfile::tempdir().unwrap();
    let fx = common::northstar(work.path()).unwrap();
    let index = ShardedIndex::open(&cluster.config()).await.unwrap();
    assert_eq!(index.home().name(), "eu");

    // Every space is new, so all of them land on the home shard and are
    // recorded there in the map.
    let report = index.reindex(work.path(), PlainFiles).await.unwrap();
    assert_eq!((report.spaces, report.documents), (3, 9));
    assert_eq!(report.problems.len(), 3);
    assert_eq!(cluster.spaces("eu").await.unwrap(), ALL_SPACES);
    assert!(cluster.spaces("us").await.unwrap().is_empty());
    assert!(cluster.spaces("main").await.unwrap().is_empty());
    assert_eq!(
        cluster
            .count(
                "main",
                "SELECT count(*) FROM space_shards WHERE shard = 'eu'"
            )
            .await
            .unwrap(),
        3
    );

    // Move Engineering to the US shard; the next reindex moves its rows.
    index.assign(fx.eng_space, "US").await.unwrap();
    assert_eq!(index.shard_for(fx.eng_space).await.unwrap().name(), "us");
    let _ = index.reindex(work.path(), PlainFiles).await.unwrap();
    assert_eq!(
        cluster.spaces("eu").await.unwrap(),
        ["tasks/ada", "teams/ENG"]
    );
    assert_eq!(
        cluster.spaces("us").await.unwrap(),
        ["drives/shared/engineering"]
    );
    let us_docs = cluster
        .count("us", "SELECT count(*) FROM documents")
        .await
        .unwrap();
    let eu_docs = cluster
        .count("eu", "SELECT count(*) FROM documents")
        .await
        .unwrap();
    assert_eq!((us_docs, eu_docs), (7, 2));
    assert_eq!(
        cluster
            .count(
                "eu",
                "SELECT count(*) FROM page_ancestors WHERE space_id IN \
             (SELECT space_id FROM spaces)"
            )
            .await
            .unwrap(),
        cluster
            .count("eu", "SELECT count(*) FROM page_ancestors")
            .await
            .unwrap(),
        "no closure rows are left behind for the moved space"
    );

    // An unknown space is on the home shard; an unknown shard is refused.
    assert_eq!(
        index.shard_for(ObjectId::generate()).await.unwrap().name(),
        "eu"
    );
    assert!(matches!(
        index.assign(fx.eng_space, "apac").await,
        Err(Error::UnknownShard(name)) if name == "apac"
    ));
    index.close().await;
}

#[tokio::test]
async fn a_map_entry_for_a_removed_shard_stops_the_reindex_before_writing() {
    let cluster = Cluster::start().await.unwrap();
    let work = tempfile::tempdir().unwrap();
    let fx = common::northstar(work.path()).unwrap();
    let index = ShardedIndex::open(&cluster.config()).await.unwrap();
    index.assign(fx.eng_space, "us").await.unwrap();
    let _ = index.reindex(work.path(), PlainFiles).await.unwrap();
    index.close().await;

    // Drop the US shard from the configuration: its spaces have nowhere to go.
    let mut config = cluster.config();
    config.shards.truncate(1);
    let index = ShardedIndex::open(&config).await.unwrap();
    assert!(matches!(
        index.reindex(work.path(), PlainFiles).await,
        Err(Error::UnknownShard(name)) if name == "us"
    ));
    assert_eq!(
        cluster.spaces("eu").await.unwrap(),
        ["tasks/ada", "teams/ENG"]
    );
    index.close().await;
}

#[tokio::test]
async fn a_moved_space_takes_all_its_rows_along() {
    let cluster = Cluster::start().await.unwrap();
    let work = tempfile::tempdir().unwrap();
    let fx = common::northstar(work.path()).unwrap();
    let index = ShardedIndex::open(&cluster.config()).await.unwrap();
    let _ = index.reindex(work.path(), PlainFiles).await.unwrap();
    // State only the database holds must move too.
    let mut eu = PgConnection::connect(&cluster.url("eu")).await.unwrap();
    let _ = sqlx::query(
        "INSERT INTO user_doc_state (space_id, user_id, doc_id, starred_at) \
         SELECT space_id, gen_random_uuid(), doc_id, 1 FROM documents WHERE space_id = $1",
    )
    .bind(fx.eng_space.as_uuid())
    .execute(&mut eu)
    .await
    .unwrap();
    let stars = "SELECT count(*) FROM user_doc_state";
    let before = (
        cluster
            .count("eu", "SELECT count(*) FROM documents")
            .await
            .unwrap(),
        cluster.count("eu", stars).await.unwrap(),
    );

    let rows = index.move_space(fx.eng_space, "us").await.unwrap();
    assert!(rows > 0);
    assert_eq!(index.shard_for(fx.eng_space).await.unwrap().name(), "us");
    assert_eq!(
        cluster.spaces("us").await.unwrap(),
        ["drives/shared/engineering"]
    );
    assert_eq!(
        cluster.spaces("eu").await.unwrap(),
        ["tasks/ada", "teams/ENG"]
    );
    let after = (
        cluster
            .count("eu", "SELECT count(*) FROM documents")
            .await
            .unwrap()
            + cluster
                .count("us", "SELECT count(*) FROM documents")
                .await
                .unwrap(),
        cluster.count("us", stars).await.unwrap(),
    );
    assert_eq!(after, before, "every document and star moved");
    assert_eq!(cluster.count("eu", stars).await.unwrap(), 0);

    // Moving again is a no-op; moving back works.
    assert_eq!(index.move_space(fx.eng_space, "us").await.unwrap(), 0);
    assert!(index.move_space(fx.eng_space, "eu").await.unwrap() > 0);
    assert_eq!(cluster.spaces("us").await.unwrap(), Vec::<String>::new());
    index.close().await;
}
