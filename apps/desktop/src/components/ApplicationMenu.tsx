import { useEffect, useId, useRef, useState } from "react";
import {
  isEditableTarget,
  isEditCommand,
  queryEditCommandAvailability,
  runEditCommand,
  type AppCommandId,
  type EditCommandAvailability,
} from "../platform/commands";
import { MenuIcon } from "./icons";
import { Tooltip } from "./Tooltip";

interface ApplicationMenuProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onCommand: (command: AppCommandId) => void;
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

export function ApplicationMenu({ open, onOpenChange, onCommand }: ApplicationMenuProps) {
  const menuId = useId();
  const rootRef = useRef<HTMLDivElement>(null);
  const menuPanelRef = useRef<HTMLDivElement>(null);
  const onOpenChangeRef = useRef(onOpenChange);
  const onCommandRef = useRef(onCommand);
  const lastEditableRef = useRef<Element | null>(null);
  const editTargetRef = useRef<Element | null>(null);
  const menuOpenRef = useRef(open);
  const [availability, setAvailability] = useState(closedAvailability);
  const [activeIndex, setActiveIndex] = useState(0);
  const flatItems = flattenItems(availability);

  useEffect(() => {
    onOpenChangeRef.current = onOpenChange;
    onCommandRef.current = onCommand;
  }, [onOpenChange, onCommand]);

  useEffect(() => {
    menuOpenRef.current = open;
  }, [open]);

  useEffect(() => {
    function trackEditableFocus(event: FocusEvent) {
      if (menuOpenRef.current) {
        return;
      }
      const target = event.target;
      if (target instanceof Element && isEditableTarget(target)) {
        lastEditableRef.current = target;
      }
    }

    document.addEventListener("focusin", trackEditableFocus);
    return () => document.removeEventListener("focusin", trackEditableFocus);
  }, []);

  useEffect(() => {
    if (!open) {
      return;
    }

    function closeMenu() {
      editTargetRef.current = null;
      setAvailability(closedAvailability);
      onOpenChangeRef.current(false);
    }

    function dispatchMenuCommand(command: AppCommandId) {
      if (isEditCommand(command)) {
        runEditCommand(command, editTargetRef.current);
        closeMenu();
        return;
      }
      onCommandRef.current(command);
      closeMenu();
    }

    function onPointerDown(event: MouseEvent) {
      if (!rootRef.current?.contains(event.target as Node)) {
        closeMenu();
      }
    }

    function onKeyDown(event: KeyboardEvent) {
      const items = flattenItems(queryEditCommandAvailability(editTargetRef.current));

      if (event.key === "Escape") {
        event.preventDefault();
        const restore = editTargetRef.current;
        closeMenu();
        if (restore instanceof HTMLElement) {
          restore.focus();
        }
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
            dispatchMenuCommand(item.id);
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

  useEffect(() => {
    if (!open) {
      return;
    }
    const items = menuPanelRef.current?.querySelectorAll<HTMLElement>('[role="menuitem"]');
    items?.[activeIndex]?.focus();
  }, [activeIndex, open]);

  return (
    <div className="application-menu" ref={rootRef}>
      <Tooltip label="Application menu" align="start">
        <button
          className="icon-button chrome-icon"
          type="button"
          aria-label="Application menu"
          aria-haspopup="menu"
          aria-expanded={open}
          aria-controls={open ? menuId : undefined}
          onClick={() => {
            if (open) {
              editTargetRef.current = null;
              setAvailability(closedAvailability);
              onOpenChange(false);
              return;
            }

            const active = document.activeElement;
            const preserved =
              active instanceof Element &&
              isEditableTarget(active) &&
              !rootRef.current?.contains(active)
                ? active
                : lastEditableRef.current;
            const target = preserved !== null && document.contains(preserved) ? preserved : null;
            editTargetRef.current = target;
            setAvailability(queryEditCommandAvailability(target));
            setActiveIndex(0);
            onOpenChange(true);
          }}
        >
          <MenuIcon />
        </button>
      </Tooltip>
      {open ? (
        <div
          ref={menuPanelRef}
          className="application-menu-panel"
          role="menu"
          id={menuId}
          aria-label="Application"
        >
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
                    onMouseDown={(event) => {
                      // Keep the preserved editable field focused for truthful Edit actions.
                      event.preventDefault();
                    }}
                    onClick={() => {
                      if (item.disabled) {
                        return;
                      }
                      if (isEditCommand(item.id)) {
                        runEditCommand(item.id, editTargetRef.current);
                        editTargetRef.current = null;
                        setAvailability(closedAvailability);
                        onOpenChange(false);
                        return;
                      }
                      onCommand(item.id);
                      editTargetRef.current = null;
                      setAvailability(closedAvailability);
                      onOpenChange(false);
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
