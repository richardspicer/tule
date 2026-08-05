import { useEffect, useId, useState, type ReactNode } from "react";

export type TooltipAlign = "start" | "center" | "end";

interface TooltipProps {
  label: string;
  align?: TooltipAlign;
  children: ReactNode;
}

export function Tooltip({ label, align = "center", children }: TooltipProps) {
  const tooltipId = useId();
  const [visible, setVisible] = useState(false);

  useEffect(() => {
    if (!visible) {
      return;
    }

    const dismiss = () => setVisible(false);

    window.addEventListener("blur", dismiss);
    window.addEventListener("pointercancel", dismiss);
    document.addEventListener("visibilitychange", dismiss);

    return () => {
      window.removeEventListener("blur", dismiss);
      window.removeEventListener("pointercancel", dismiss);
      document.removeEventListener("visibilitychange", dismiss);
    };
  }, [visible]);

  return (
    <span
      className="tooltip-anchor"
      data-tooltip-align={align}
      onMouseEnter={() => setVisible(true)}
      onMouseLeave={() => setVisible(false)}
      onFocusCapture={() => setVisible(true)}
      onBlurCapture={() => setVisible(false)}
    >
      {children}
      {visible ? (
        <span className="tooltip" role="tooltip" id={tooltipId}>
          {label}
        </span>
      ) : null}
    </span>
  );
}
