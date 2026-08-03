import { useState } from "react";
import type { AppCommandId } from "../platform/commands";
import { ApplicationMenu } from "./ApplicationMenu";
import { SettingsIcon } from "./icons";
import { Tooltip } from "./Tooltip";

interface ApplicationChromeProps {
  onCommand: (command: AppCommandId) => void;
}

export function ApplicationChrome({ onCommand }: ApplicationChromeProps) {
  const [menuOpen, setMenuOpen] = useState(false);

  return (
    <header className="application-chrome">
      <ApplicationMenu open={menuOpen} onOpenChange={setMenuOpen} onCommand={onCommand} />

      <Tooltip label="Settings" align="end">
        <button
          className="icon-button chrome-icon"
          type="button"
          aria-label="Settings"
          onClick={() => onCommand("open-settings")}
        >
          <SettingsIcon />
        </button>
      </Tooltip>
    </header>
  );
}
