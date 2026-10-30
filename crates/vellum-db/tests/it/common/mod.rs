// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! A small Northstar workspace on disk, shared by the reindex tests.

use std::fs;
use std::io;
use std::path::Path;

use vellum_kernel::ObjectId;

/// IDs of the objects the fixture writes, for assertions.
pub struct Fixture {
    pub eng_space: ObjectId,
    pub specs_folder: ObjectId,
    pub roadmap: ObjectId,
    pub handbook: ObjectId,
    pub handbook_intro: ObjectId,
}

fn write(root: &Path, rel: &str, body: &str) -> io::Result<()> {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap_or(root))?;
    fs::write(path, body)
}

fn page(id: ObjectId, title: &str) -> String {
    format!(
        "---\nid: {id}\ntitle: {title}\nstatus: verified\nvellum: 1\n---\n\n# {title}\n\nNorthstar notes.\n"
    )
}

/// Writes the workspace and returns the IDs it used.
///
/// Also writes three things a reindex must skip and report: a page with no
/// `id`, a copy of the roadmap with the same `id`, and a folder without a
/// manifest (whose page is still indexed, under the space).
pub fn northstar(root: &Path) -> io::Result<Fixture> {
    let id = ObjectId::generate;
    let f = Fixture {
        eng_space: id(),
        specs_folder: id(),
        roadmap: id(),
        handbook: id(),
        handbook_intro: id(),
    };
    let (team, issue, tasks, row, collection) = (id(), id(), id(), id(), id());

    write(root, ".vellum/trash/old.md", &page(id(), "Ignored"))?;
    write(
        root,
        "drives/shared/engineering/.vellum-folder.yaml",
        &format!("id: {}\nname: Engineering\n", f.eng_space),
    )?;
    write(
        root,
        "drives/shared/engineering/specs/.vellum-folder.yaml",
        &format!("id: {}\nicon: 📐\n", f.specs_folder),
    )?;
    write(
        root,
        "drives/shared/engineering/specs/roadmap.md",
        &page(f.roadmap, "Roadmap 2027"),
    )?;
    write(
        root,
        "drives/shared/engineering/specs/zz-roadmap-copy.md",
        &page(f.roadmap, "Roadmap 2027"),
    )?;
    write(
        root,
        "drives/shared/engineering/specs/untitled.md",
        "# No front matter\n",
    )?;
    write(
        root,
        "drives/shared/engineering/loose/notes.md",
        &page(id(), "Loose notes"),
    )?;
    write(
        root,
        "drives/shared/engineering/handbook.book/book.yaml",
        &format!("id: {}\ntitle: Handbook\n", f.handbook),
    )?;
    write(
        root,
        "drives/shared/engineering/handbook.book/SUMMARY.md",
        "# Summary\n\n- [Intro](intro.md)\n",
    )?;
    write(
        root,
        "drives/shared/engineering/handbook.book/intro.md",
        &page(f.handbook_intro, "Intro"),
    )?;
    write(
        root,
        "drives/shared/engineering/okrs.collection/schema.yaml",
        &format!("id: {collection}\ntitle: OKRs\n"),
    )?;
    write(
        root,
        "drives/shared/engineering/okrs.collection/rows/q1.md",
        &page(row, "Q1"),
    )?;
    write(
        root,
        "drives/shared/engineering/go.shortcut.yaml",
        &format!("id: {}\ntarget: {}\n", id(), f.roadmap),
    )?;
    write(
        root,
        "teams/ENG/.vellum-folder.yaml",
        &format!("id: {team}\nname: Engineering team\n"),
    )?;
    write(
        root,
        "teams/ENG/issues/ENG-1.md",
        &page(issue, "Ship the index"),
    )?;
    write(
        root,
        "tasks/ada/.vellum-folder.yaml",
        &format!("id: {tasks}\n"),
    )?;
    write(root, "tasks/ada/today.md", &page(id(), "Today"))?;
    Ok(f)
}
