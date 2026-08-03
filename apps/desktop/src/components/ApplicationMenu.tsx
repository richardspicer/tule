import { useEffect, useId, useRef, useState } from "react";
import {
  queryEditCommandAvailability,
  type AppCommandId,
  type EditCommandAvailability,
} from "../platform/commands";

interface ApplicationMenuProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onCommand: (command: AppCommandId) => void;
  trigger: React.ReactNode;
}

interface MenuItem {
  id: AppCommandId;
  label: string;
  shortcut?: string;
  editKey?: keyof EditCommandAvailability;
}

interface MenuGroup {
  label: string;
  items: readonly MenuItem[];
}

const menuGroups: readonly MenuGroup[] = [
  {
    label: "File",
    items: [
      { id: "new-session", label: "New session" },
      { id: "manage-projects", label: "Manage projects" },
      { id: "exit", label: "Exit" },
    ],
  },
  {
    label: "Edit",
    items: [
      { id: "edit-undo", label: "Undo", editKey: "undo" },
      { id: "edit-redo", label: "Redo", editKey: "redo" },
      { id: "edit-cut", label: "Cut", editKey: "cut" },
      { id: "edit-copy", label: "Copy", editKey: "copy" },
      { id: "edit-paste", label: "Paste", editKey: "paste" },
      { id: "edit-select-all", label: "Select all", editKey: "selectAll" },
    ],
  },
  {
    label: "Settings",
    items: [{ id: "open-settings", label: "Open Settings", shortcut: "Ctrl+," }],
  },
];

const closedAvailability: EditCommandAvailability = {
  undo: false,
  redo: false,
  cut: false,
  copy: false,
  paste: false,
  selectAll: false,
};

function flattenItems(availability: EditCommandAvailability) {
  return menuGroups.flatMap((group) =>
    group.items.map((item) => ({
      group: group.label,
      ...item,
      disabled: item.editKey !== undefined ? !availability[item.editKey] : false,
    })),
  );
}

export function ApplicationMenu({ open, onOpenChange, onCommand, trigger }: ApplicationMenuProps) {
  const menuId = useId();
  const rootRef = useRef<HTMLDivElement>(null);
  const onOpenChangeRef = useRef(onOpenChange);
  const onCommandRef = useRef(onCommand);
  const [activeIndex, setActiveIndex] = useState(0);
  const availability = open ? queryEditCommandAvailability() : closedAvailability;
  const flatItems = flattenItems(availability);

  useEffect(() => {
    onOpenChangeRef.current = onOpenChange;
    onCommandRef.current = onCommand;
  }, [onOpenChange, onCommand]);

  useEffect(() => {
    if (!open) {
      return;
    }

    function onPointerDown(event: MouseEvent) {
      if (!rootRef.current?.contains(event.target as Node)) {
        onOpenChangeRef.current(false);
      }
    }

    function onKeyDown(event: KeyboardEvent) {
      const items = flattenItems(queryEditCommandAvailability());

      if (event.key === "Escape") {
        event.preventDefault();
        onOpenChangeRef.current(false);
        return;
      }

      if (event.key === "ArrowDown") {
        event.preventDefault();
        setActiveIndex((current) => (current + 1) % items.length);
        return;
      }

      if (event.key === "ArrowUp") {
        event.preventDefault();
        setActiveIndex((current) => (current - 1 + items.length) % items.length);
        return;
      }

      if (event.key === "Enter") {
        event.preventDefault();
        setActiveIndex((current) => {
          const item = items[current];
          if (item !== undefined && !item.disabled) {
            onCommandRef.current(item.id);
            onOpenChangeRef.current(false);
          }
          return current;
        });
      }
    }

    window.addEventListener("mousedown", onPointerDown);
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("mousedown", onPointerDown);
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [open]);

  return (
    <div className="application-menu" ref={rootRef}>
      {trigger}
      {open ? (
        <div className="application-menu-panel" role="menu" id={menuId} aria-label="Application">
          {menuGroups.map((group) => (
            <div
              key={group.label}
              className="application-menu-group"
              role="group"
              aria-label={group.label}
            >
              <div className="application-menu-heading">{group.label}</div>
              {flatItems
                .map((item, flatIndex) => ({ item, flatIndex }))
                .filter(({ item }) => item.group === group.label)
                .map(({ item, flatIndex }) => (
                  <button
                    key={item.id}
                    className={`application-menu-item${flatIndex === activeIndex ? " is-active" : ""}`}
                    type="button"
                    role="menuitem"
                    disabled={item.disabled}
                    tabIndex={flatIndex === activeIndex ? 0 : -1}
                    onMouseEnter={() => setActiveIndex(flatIndex)}
                    onClick={() => {
                      if (!item.disabled) {
                        onCommand(item.id);
                        onOpenChange(false);
                      }
                    }}
                  >
                    <span>{item.label}</span>
                    {item.shortcut === undefined ? null : (
                      <span className="application-menu-shortcut">{item.shortcut}</span>
                    )}
                  </button>
                ))}
            </div>
          ))}
        </div>
      ) : null}
    </div>
  );
}
