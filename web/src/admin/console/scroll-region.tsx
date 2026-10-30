// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { ReactNode } from "react";

/**
 * A table's box that scrolls sideways on a narrow screen. A region that
 * scrolls must take keyboard focus, or keyboard users cannot reach the
 * columns it hides (WCAG 2.1.1, axe `scrollable-region-focusable`), so it
 * is a named, focusable region.
 */
export function ScrollRegion({
  label,
  children,
}: {
  readonly label: string;
  readonly children: ReactNode;
}) {
  return (
    // eslint-disable-next-line jsx-a11y/no-noninteractive-tabindex -- see above
    <div className="vl-admin-scroll" tabIndex={0} role="region" aria-label={label}>
      {children}
    </div>
  );
}
