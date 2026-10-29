// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useEffect, useRef, type ReactNode } from "react";

interface PopoverProperties {
  readonly label: string;
  readonly onClose: () => void;
  readonly children: ReactNode;
  readonly className?: string;
}

/**
 * A non-modal dialog anchored by CSS. Escape and a click outside close it,
 * and focus returns to whatever opened it.
 */
export function Popover({ label, onClose, children, className = "" }: PopoverProperties) {
  const panel = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const opener = document.activeElement as HTMLElement | null;
    panel.current?.querySelector<HTMLElement>("input, button, a, [tabindex]")?.focus();
    const onPointer = (event: PointerEvent) => {
      if (!panel.current?.contains(event.target as Node)) {
        onClose();
      }
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        onClose();
      }
    };
    document.addEventListener("pointerdown", onPointer);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("pointerdown", onPointer);
      document.removeEventListener("keydown", onKey);
      opener?.focus();
    };
  }, [onClose]);
  return (
    <div className={`vl-popover ${className}`} role="dialog" aria-label={label} ref={panel}>
      {children}
    </div>
  );
}
