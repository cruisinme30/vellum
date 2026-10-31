// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useId, useState, type SyntheticEvent, type ReactNode } from "react";
import { Link, NavLink, useParams } from "react-router";

import { failureText } from "../data/failure";
import { Ready, ServerScreen } from "../data/server-apis";
import { useLoaded } from "../data/use-loaded";
import { useT } from "../i18n/i18n";
import { CopyDiagnostic } from "../shell/copy-diagnostic";

import type { DriveApi, NodeView, SpaceView } from "./api";
import { ActivityFeed } from "./details-panel";
import { ItemTable } from "./item-table";
import {
  ancestors,
  loadSpaces,
  useReloadable,
  withStars,
  type Listing,
} from "./loading";
import {
  DRIVE_HOME,
  DRIVE_SEARCH,
  RECENT,
  SHARED_DRIVES,
  SHARED_WITH_ME,
  STARRED,
  folderPath,
  settingsPath,
  spacePath,
  trashPath,
} from "./paths";
import { UsageMeter } from "./usage";

import "./drive.css";

const PLACES = [
  { to: DRIVE_HOME, label: "drive-my-drive", end: true },
  { to: SHARED_DRIVES, label: "drive-shared-drives", end: true },
  { to: SHARED_WITH_ME, label: "drive-shared-with-me", end: true },
  { to: STARRED, label: "drive-starred", end: true },
  { to: RECENT, label: "drive-recent", end: true },
  { to: DRIVE_SEARCH, label: "drive-search", end: true },
] as const;

/**
 * The Drive's places: My Drive, Shared drives, Shared with me, Starred, Recent
 * and Search (R-DR-1).
 */
function DriveNav() {
  const t = useT();
  return (
    <nav className="vl-drive-nav" aria-label={t("drive")}>
      {PLACES.map((place) => (
        <NavLink key={place.to} to={place.to} end={place.end}>
          {t(place.label)}
        </NavLink>
      ))}
    </nav>
  );
}

/** A Drive screen: the places, then the screen, or the needs-a-server note. */
export function DriveFrame({
  children,
}: {
  readonly children: (api: DriveApi) => ReactNode;
}) {
  const t = useT();
  return (
    <ServerScreen api="drive" title={t("drive")} className="vl-drive">
      {(api) => (
        <>
          <DriveNav />
          {children(api)}
        </>
      )}
    </ServerScreen>
  );
}

/** A new folder's name, made in the folder or space on view. */
function NewFolder({
  api,
  spaceId,
  parentId,
  onMade,
}: {
  readonly api: DriveApi;
  readonly spaceId: string;
  readonly parentId: string | undefined;
  readonly onMade: () => void;
}) {
  const t = useT();
  const id = useId();
  const [name, setName] = useState("");
  const [problem, setProblem] = useState<string>();
  const submit = async (event: SyntheticEvent) => {
    event.preventDefault();
    const trimmed = name.trim();
    if (trimmed === "") return;
    try {
      await api.createFolder(spaceId, parentId, trimmed);
      setName("");
      setProblem(undefined);
      onMade();
    } catch (error) {
      setProblem(failureText(error));
    }
  };
  return (
    <form
      className="vl-drive-new-folder"
      onSubmit={(event) => void submit(event)}
    >
      <label htmlFor={id}>{t("drive-new-folder")}</label>
      <input
        id={id}
        value={name}
        onChange={(event) => {
          setName(event.target.value);
        }}
      />
      <button type="submit" className="vl-text-button">
        {t("drive-create")}
      </button>
      {problem !== undefined && (
        <span className="vl-field-error" role="alert">
          {problem}
          <CopyDiagnostic message={problem} />
        </span>
      )}
    </form>
  );
}

function Breadcrumbs({
  api,
  space,
  folderId,
}: {
  readonly api: DriveApi;
  readonly space: SpaceView;
  readonly folderId: string | undefined;
}) {
  const t = useT();
  const loaded = useLoaded(
    () =>
      folderId === undefined ? Promise.resolve([]) : ancestors(api, folderId),
    [api, folderId],
    failureText,
  );
  const chain = loaded.state === "ready" ? loaded.value : [];
  return (
    <nav className="vl-drive-crumbs" aria-label={t("drive-breadcrumbs")}>
      <ol>
        <li>
          <Link
            to={spacePath(space.id)}
            aria-current={folderId === undefined ? "page" : undefined}
          >
            {space.name}
          </Link>
        </li>
        {chain.map((node) => (
          <li key={node.id}>
            <Link
              to={folderPath(space.id, node.id)}
              aria-current={node.id === folderId ? "page" : undefined}
            >
              {node.title}
            </Link>
          </li>
        ))}
      </ol>
    </nav>
  );
}

/**
 * A space's top or one of its folders, with its tools and the space's usage.
 */
function SpaceContents({
  api,
  spaces,
  space,
  folderId,
}: {
  readonly api: DriveApi;
  readonly spaces: readonly SpaceView[];
  readonly space: SpaceView;
  readonly folderId: string | undefined;
}) {
  const t = useT();
  const [showActivity, setShowActivity] = useState(false);
  const { loaded, reload } = useReloadable(
    () => withStars(api, api.list(space.id, folderId)),
    [api, space.id, folderId],
  );
  return (
    <>
      <div className="vl-drive-head">
        <Breadcrumbs api={api} space={space} folderId={folderId} />
        <div className="vl-drive-tools">
          <button
            type="button"
            className="vl-text-button"
            aria-expanded={showActivity}
            onClick={() => {
              setShowActivity((shown) => !shown);
            }}
          >
            {t("drive-space-activity")}
          </button>
          <Link className="vl-text-button" to={trashPath(space.id)}>
            {t("drive-trash-open")}
          </Link>
          <Link className="vl-text-button" to={settingsPath(space.id)}>
            {t("drive-settings")}
          </Link>
        </div>
      </div>
      <UsageMeter api={api} space={space} />
      {showActivity && (
        <section
          className="vl-drive-space-activity"
          aria-label={t("drive-space-activity")}
        >
          <ActivityFeed api={api} id={space.id} />
        </section>
      )}
      <NewFolder
        api={api}
        spaceId={space.id}
        parentId={folderId}
        onMade={reload}
      />
      <Ready loaded={loaded}>
        {(listing) => (
          <ItemTable
            api={api}
            items={listing.items}
            starred={listing.starred}
            spaces={spaces}
            empty={t("drive-folder-empty")}
            onChanged={reload}
          />
        )}
      </Ready>
    </>
  );
}

/** Loads the spaces, then shows `children` with them. */
function WithSpaces({
  api,
  children,
}: {
  readonly api: DriveApi;
  readonly children: (spaces: SpaceView[]) => ReactNode;
}) {
  const loaded = useLoaded(() => loadSpaces(api), [api], failureText);
  return <Ready loaded={loaded}>{children}</Ready>;
}

/** My Drive: the person's own space (R-DR-1). */
export function MyDrivePage() {
  const t = useT();
  return (
    <DriveFrame>
      {(api) => (
        <WithSpaces api={api}>
          {(spaces) => {
            const mine = spaces.find((space) => space.kind === "user_drive");
            return mine === undefined ? (
              <p className="vl-muted">{t("drive-no-my-drive")}</p>
            ) : (
              <SpaceContents
                api={api}
                spaces={spaces}
                space={mine}
                folderId={undefined}
              />
            );
          }}
        </WithSpaces>
      )}
    </DriveFrame>
  );
}

/** A space, or a folder in it, by the route's IDs. */
export function SpacePage() {
  const t = useT();
  const { spaceId = "", folderId } = useParams();
  return (
    <DriveFrame>
      {(api) => (
        <WithSpaces api={api}>
          {(spaces) => {
            const space = spaces.find((candidate) => candidate.id === spaceId);
            return space === undefined ? (
              <p className="vl-muted">{t("drive-space-missing")}</p>
            ) : (
              <SpaceContents
                key={space.id}
                api={api}
                spaces={spaces}
                space={space}
                folderId={folderId}
              />
            );
          }}
        </WithSpaces>
      )}
    </DriveFrame>
  );
}

/** Shared drives the person can open, each with its usage. */
export function SharedDrivesPage() {
  const t = useT();
  return (
    <DriveFrame>
      {(api) => (
        <WithSpaces api={api}>
          {(spaces) => {
            const shared = spaces.filter(
              (space) => space.kind === "shared_drive",
            );
            if (shared.length === 0)
              return <p className="vl-muted">{t("drive-no-shared")}</p>;
            return (
              <ul className="vl-drive-spaces">
                {shared.map((space) => (
                  <li key={space.id}>
                    <Link to={spacePath(space.id)}>{space.name}</Link>
                    <UsageMeter api={api} space={space} />
                  </li>
                ))}
              </ul>
            );
          }}
        </WithSpaces>
      )}
    </DriveFrame>
  );
}

/** A flat list of items from one load: shared with me, starred or recent. */
function ListPage({
  load,
  empty,
}: {
  readonly load: (api: DriveApi) => Promise<NodeView[]>;
  readonly empty: string;
}) {
  return (
    <DriveFrame>
      {(api) => <FlatList api={api} load={load} empty={empty} />}
    </DriveFrame>
  );
}

function FlatList({
  api,
  load,
  empty,
}: {
  readonly api: DriveApi;
  readonly load: (api: DriveApi) => Promise<NodeView[]>;
  readonly empty: string;
}) {
  const { loaded, reload } = useReloadable<[Listing, SpaceView[]]>(
    () => Promise.all([withStars(api, load(api)), loadSpaces(api)]),
    [api, load],
  );
  return (
    <Ready loaded={loaded}>
      {([listing, spaces]) => (
        <ItemTable
          api={api}
          items={listing.items}
          starred={listing.starred}
          spaces={spaces}
          empty={empty}
          onChanged={reload}
        />
      )}
    </Ready>
  );
}

const sharedWithMe = (api: DriveApi) => api.sharedWithMe();
const starred = (api: DriveApi) => api.starred();
const recent = (api: DriveApi) => api.recent();

/** Items in other people's My Drives shared with the person. */
export function SharedWithMePage() {
  const t = useT();
  return (
    <ListPage load={sharedWithMe} empty={t("drive-shared-with-me-empty")} />
  );
}

/** Items the person starred. */
export function StarredPage() {
  const t = useT();
  return <ListPage load={starred} empty={t("drive-starred-empty")} />;
}

/** Items the person opened lately, newest first. */
export function RecentPage() {
  const t = useT();
  return <ListPage load={recent} empty={t("drive-recent-empty")} />;
}
