// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import {
  useMemo,
  useRef,
  useState,
  type DragEvent,
  type KeyboardEvent,
} from "react";
import { useNavigate } from "react-router";

import {
  ancestors,
  buildTree,
  targetFor,
  type TreeMove,
  type TreeNode,
} from "../data/tree";
import type { MoveTarget, PageMeta, Space } from "../data/types";
import { useServices } from "../data/workspace-context";
import { useT } from "../i18n/i18n";

import { Icon } from "./icon";
import { PageMenu } from "./page-menu";

interface Row {
  readonly node: TreeNode;
  readonly level: number;
  readonly parentId: string | undefined;
  readonly index: number;
  readonly size: number;
}

/**
 * The rows a keyboard user can reach: every page whose ancestors are expanded.
 */
export function visibleRows(
  nodes: readonly TreeNode[],
  expanded: ReadonlySet<string>,
): Row[] {
  const rows: Row[] = [];
  const walk = (
    list: readonly TreeNode[],
    level: number,
    parentId?: string,
  ) => {
    for (const [index, node] of list.entries()) {
      rows.push({ node, level, parentId, index, size: list.length });
      if (expanded.has(node.page.id)) {
        walk(node.children, level + 1, node.page.id);
      }
    }
  };
  walk(nodes, 1);
  return rows;
}

const KEY_MOVES: Readonly<Record<string, TreeMove>> = {
  ArrowUp: "up",
  ArrowDown: "down",
  ArrowRight: "indent",
  ArrowLeft: "outdent",
};

/** Where a drop lands: before, inside or after the row under the pointer. */
export function dropTarget(
  row: Row,
  spaceId: string,
  offset: number,
  height: number,
): MoveTarget {
  if (offset < height * 0.25) {
    return { spaceId, parentId: row.parentId, index: row.index };
  }
  if (offset > height * 0.75) {
    return { spaceId, parentId: row.parentId, index: row.index + 1 };
  }
  return {
    spaceId,
    parentId: row.node.page.id,
    index: row.node.children.length,
  };
}

interface PageTreeProperties {
  readonly space: Space;
  readonly pages: readonly PageMeta[];
  readonly currentId: string | undefined;
  readonly onNavigate?: () => void;
}

/**
 * A space's page tree: a WAI-ARIA tree with one tab stop. Arrows move and
 * expand, Enter opens, Alt+Shift+arrows reorder and nest, and Shift+F10
 * opens the page menu, so dragging always has a keyboard alternative
 * (WCAG 2.5.7).
 */
export function PageTree({
  space,
  pages,
  currentId,
  onNavigate,
}: PageTreeProperties) {
  const t = useT();
  const navigate = useNavigate();
  const { store } = useServices();
  const tree = useMemo(() => buildTree(pages, space.id), [pages, space.id]);
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(
    () =>
      new Set([
        ...pages.filter((p) => p.parentId === undefined).map((p) => p.id),
        ...(currentId ? ancestors(pages, currentId).map((p) => p.id) : []),
      ]),
  );
  const [focusId, setFocusId] = useState<string>();
  const [menuFor, setMenuFor] = useState<string>();
  const [renaming, setRenaming] = useState<string>();
  const [dragging, setDragging] = useState<string>();
  const list = useRef<HTMLDivElement>(null);
  const rows = visibleRows(tree, expanded);
  const tabStop =
    rows.find((r) => r.node.page.id === (focusId ?? currentId))?.node.page.id ??
    rows[0]?.node.page.id;

  const focus = (id: string | undefined) => {
    if (id === undefined) {
      return;
    }
    setFocusId(id);
    list.current?.querySelector<HTMLElement>(`[data-page-id="${id}"]`)?.focus();
  };
  const toggle = (id: string, open?: boolean) => {
    setExpanded((current) => {
      const next = new Set(current);
      if (open ?? !next.has(id)) {
        next.add(id);
      } else {
        next.delete(id);
      }
      return next;
    });
  };
  const open = (id: string) => {
    void navigate(`/p/${id}`);
    onNavigate?.();
  };
  const move = (id: string, how: TreeMove) => {
    const target = targetFor(pages, id, how);
    if (target) {
      if (target.parentId !== undefined) {
        toggle(target.parentId, true);
      }
      store.movePage(id, target);
      requestAnimationFrame(() => {
        focus(id);
      });
    }
  };

  const onKeyDown = (event: KeyboardEvent, row: Row, at: number) => {
    const id = row.node.page.id;
    const hasChildren = row.node.children.length > 0;
    const reorder =
      event.altKey && event.shiftKey ? KEY_MOVES[event.key] : undefined;
    if (reorder) {
      event.preventDefault();
      move(id, reorder);
      return;
    }
    if (event.key === "F10" && event.shiftKey) {
      event.preventDefault();
      setMenuFor(id);
      return;
    }
    const actions: Record<string, () => void> = {
      ArrowDown: () => {
        focus(rows[at + 1]?.node.page.id);
      },
      ArrowUp: () => {
        focus(rows[at - 1]?.node.page.id);
      },
      Home: () => {
        focus(rows[0]?.node.page.id);
      },
      End: () => {
        focus(rows.at(-1)?.node.page.id);
      },
      ArrowRight: () => {
        if (hasChildren && !expanded.has(id)) {
          toggle(id, true);
        } else if (hasChildren) {
          focus(row.node.children[0]?.page.id);
        }
      },
      ArrowLeft: () => {
        if (hasChildren && expanded.has(id)) {
          toggle(id, false);
        } else {
          focus(row.parentId);
        }
      },
      Enter: () => {
        open(id);
      },
      F2: () => {
        setRenaming(id);
      },
    };
    const action = actions[event.key];
    if (action) {
      event.preventDefault();
      action();
    }
  };

  const onDrop = (event: DragEvent<HTMLElement>, row: Row) => {
    event.preventDefault();
    const id =
      dragging ?? event.dataTransfer.getData("application/x-vellum-page");
    const box = event.currentTarget.getBoundingClientRect();
    if (id !== "" && id !== row.node.page.id) {
      const target = dropTarget(
        row,
        space.id,
        event.clientY - box.top,
        box.height,
      );
      if (target.parentId !== undefined) {
        toggle(target.parentId, true);
      }
      store.movePage(id, target);
    }
    setDragging(undefined);
  };

  return (
    <div
      className="vl-tree"
      role="tree"
      aria-label={t("tree-label", { space: space.name })}
      ref={list}
    >
      {rows.map((row, at) => {
        const { page, children } = row.node;
        const isOpen = expanded.has(page.id);
        const title = page.title === "" ? t("page-untitled") : page.title;
        return (
          <div
            key={page.id}
            role="treeitem"
            data-page-id={page.id}
            aria-level={row.level}
            aria-setsize={row.size}
            aria-posinset={row.index + 1}
            aria-expanded={children.length > 0 ? isOpen : undefined}
            aria-selected={page.id === currentId}
            aria-current={page.id === currentId ? "page" : undefined}
            tabIndex={page.id === tabStop ? 0 : -1}
            className="vl-tree-row"
            style={{ paddingInlineStart: `${String(row.level * 12 - 4)}px` }}
            draggable
            onDragStart={(event) => {
              setDragging(page.id);
              event.dataTransfer.setData("application/x-vellum-page", page.id);
              event.dataTransfer.effectAllowed = "move";
            }}
            onDragOver={(event) => {
              event.preventDefault();
            }}
            onDrop={(event) => {
              onDrop(event, row);
            }}
            onFocus={(event) => {
              if (event.target === event.currentTarget) {
                setFocusId(page.id);
              }
            }}
            onKeyDown={(event) => {
              if (event.target === event.currentTarget) {
                onKeyDown(event, row, at);
              }
            }}
            onClick={() => {
              open(page.id);
            }}
          >
            {children.length > 0 ? (
              <button
                type="button"
                className="vl-tree-twisty"
                tabIndex={-1}
                aria-label={t(isOpen ? "tree-collapse" : "tree-expand", {
                  title,
                })}
                onClick={(event) => {
                  event.stopPropagation();
                  toggle(page.id);
                }}
              >
                <Icon
                  name={isOpen ? "keyboard_arrow_down" : "chevron_right"}
                  size={18}
                />
              </button>
            ) : (
              <span className="vl-tree-twisty" />
            )}
            <span className="vl-tree-icon" aria-hidden="true">
              {page.icon}
            </span>
            {renaming === page.id ? (
              <input
                className="vl-inline-input"
                aria-label={t("tree-rename")}
                defaultValue={page.title}
                // Moving focus into the field the user just asked to edit.
                // eslint-disable-next-line jsx-a11y/no-autofocus
                autoFocus
                onClick={(event) => {
                  event.stopPropagation();
                }}
                onKeyDown={(event) => {
                  event.stopPropagation();
                  if (event.key === "Enter" || event.key === "Escape") {
                    if (event.key === "Escape") {
                      // Moving focus blurs the field, and blurring saves.
                      event.currentTarget.value = page.title;
                    }
                    setRenaming(undefined);
                    focus(page.id);
                  }
                }}
                onBlur={(event) => {
                  const title = event.currentTarget.value.trim();
                  if (title !== page.title) {
                    store.renamePage(page.id, title);
                  }
                  setRenaming(undefined);
                }}
              />
            ) : (
              <span className="vl-tree-title">{title}</span>
            )}
            <PageMenu
              page={page}
              open={menuFor === page.id}
              onOpenChange={(next) => {
                setMenuFor(next ? page.id : undefined);
                if (!next) {
                  focus(page.id);
                }
              }}
              onRename={() => {
                setRenaming(page.id);
              }}
              onMove={(how) => {
                move(page.id, how);
              }}
              onAddChild={() => {
                void store
                  .createPage({ spaceId: space.id, parentId: page.id })
                  .then((child) => {
                    if (child === undefined) return;
                    toggle(page.id, true);
                    open(child.id);
                  });
              }}
            />
          </div>
        );
      })}
    </div>
  );
}
