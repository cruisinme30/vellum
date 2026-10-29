-- SPDX-FileCopyrightText: 2026 The Vellum contributors
-- SPDX-License-Identifier: AGPL-3.0-only

-- Search index for the HA profile (D23). Owned by the search service, in its
-- own schema; rebuilt from the files like every other index. One row per
-- block (the retrieval unit) and one per document (instant title search).
-- IDs are kernel IDs; block IDs are 40-bit integers; times are epoch ms.

CREATE EXTENSION IF NOT EXISTS pg_trgm;

CREATE TABLE vellum_search.blocks (
    space_id UUID NOT NULL,
    doc_id UUID NOT NULL,
    block_id BIGINT NOT NULL,
    position INTEGER NOT NULL,
    section_id BIGINT NULL,
    doc_kind TEXT NOT NULL,
    block_kind TEXT NOT NULL,
    title TEXT NOT NULL,
    path TEXT NOT NULL,
    headings TEXT [] NOT NULL,
    headings_text TEXT NOT NULL,
    body TEXT NOT NULL,
    owner_id UUID NULL,
    modified_at BIGINT NOT NULL,
    ancestors UUID [] NOT NULL,
    trashed BOOLEAN NOT NULL,
    embedding REAL [] NULL,
    tsv TSVECTOR GENERATED ALWAYS AS (
        setweight(to_tsvector('english'::REGCONFIG, title), 'A')
        || setweight(to_tsvector('english'::REGCONFIG, headings_text), 'B')
        || setweight(to_tsvector('english'::REGCONFIG, body), 'D')
    ) STORED,
    PRIMARY KEY (doc_id, block_id)
);

CREATE INDEX blocks_tsv ON vellum_search.blocks USING gin (tsv);
CREATE INDEX blocks_scope ON vellum_search.blocks (space_id, trashed);
CREATE INDEX blocks_ancestors ON vellum_search.blocks USING gin (ancestors);
CREATE INDEX blocks_section ON vellum_search.blocks (
    doc_id, section_id, position
);

CREATE TABLE vellum_search.docs (
    space_id UUID NOT NULL,
    doc_id UUID NOT NULL PRIMARY KEY,
    doc_kind TEXT NOT NULL,
    title TEXT NOT NULL,
    path TEXT NOT NULL,
    owner_id UUID NULL,
    modified_at BIGINT NOT NULL,
    ancestors UUID [] NOT NULL,
    trashed BOOLEAN NOT NULL
);

CREATE INDEX docs_title_trgm ON vellum_search.docs USING gin (
    lower(title) gin_trgm_ops
);
CREATE INDEX docs_scope ON vellum_search.docs (space_id, trashed);

-- Cosine similarity of unit vectors. Exact scan over the filtered rows;
-- embeddings are optional and no vector extension is required.
CREATE FUNCTION vellum_search.dot(a REAL [], b REAL [])
RETURNS REAL
LANGUAGE sql
IMMUTABLE STRICT PARALLEL SAFE
AS $$
    SELECT sum(x * y)::REAL FROM unnest(a, b) AS t (x, y)
$$;
