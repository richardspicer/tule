import {
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
  type ReactNode,
} from "react";

export type TooltipAlign = "start" | "center" | "end";
export type TooltipPlacement = "above" | "below";

interface TooltipProps {
  label: string;
  align?: TooltipAlign;
  children: ReactNode;
}

const VIEWPORT_MARGIN = 8;
const TOOLTIP_GAP = 6;

interface ComputedPlacement {
  vertical: TooltipPlacement;
  left: number;
}

function computePlacement(
  anchorRect: DOMRect,
  tooltipRect: DOMRect,
  align: TooltipAlign,
): ComputedPlacement {
  const spaceBelow = window.innerHeight - anchorRect.bottom - TOOLTIP_GAP;
  const spaceAbove = anchorRect.top - TOOLTIP_GAP;

  let vertical: TooltipPlacement = "below";
  if (tooltipRect.height > spaceBelow) {
    if (tooltipRect.height <= spaceAbove || spaceAbove >= spaceBelow) {
      vertical = "above";
    }
  }

  let tooltipLeft: number;
  switch (align) {
    case "start":
      tooltipLeft = anchorRect.left;
      break;
    case "end":
      tooltipLeft = anchorRect.right - tooltipRect.width;
      break;
    default:
      tooltipLeft = anchorRect.left + (anchorRect.width - tooltipRect.width) / 2;
  }

  const maxLeft = window.innerWidth - tooltipRect.width - VIEWPORT_MARGIN;
  tooltipLeft = Math.max(VIEWPORT_MARGIN, Math.min(tooltipLeft, maxLeft));

  return {
    vertical,
    left: tooltipLeft - anchorRect.left,
  };
}

export function Tooltip({ label, align = "center", children }: TooltipProps) {
  const tooltipId = useId();
  const [visible, setVisible] = useState(false);
  const [placement, setPlacement] = useState<ComputedPlacement | null>(null);
  const anchorRef = useRef<HTMLSpanElement>(null);
  const tooltipRef = useRef<HTMLSpanElement>(null);

  useLayoutEffect(() => {
    if (!visible) {
      return;
    }

    const anchor = anchorRef.current;
    const tooltip = tooltipRef.current;
    if (!anchor || !tooltip) {
      return;
    }

    setPlacement(
      computePlacement(anchor.getBoundingClientRect(), tooltip.getBoundingClientRect(), align),
    );
  }, [visible, align, label]);

  const show = () => {
    setPlacement(null);
    setVisible(true);
  };

  const hide = () => {
    setVisible(false);
    setPlacement(null);
  };

  useEffect(() => {
    if (!visible) {
      return;
    }

    window.addEventListener("blur", hide);
    window.addEventListener("pointercancel", hide);
    document.addEventListener("visibilitychange", hide);

    return () => {
      window.removeEventListener("blur", hide);
      window.removeEventListener("pointercancel", hide);
      document.removeEventListener("visibilitychange", hide);
    };
  }, [visible]);

  const tooltipStyle: CSSProperties | undefined =
    placement === null
      ? undefined
      : {
          left: `${placement.left}px`,
          right: "auto",
          transform: "none",
        };

  return (
    <span
      ref={anchorRef}
      className="tooltip-anchor"
      data-tooltip-align={align}
      data-tooltip-placement={placement?.vertical}
      onMouseEnter={show}
      onMouseLeave={hide}
      onFocusCapture={show}
      onBlurCapture={hide}
    >
      {children}
      {visible ? (
        <span
          ref={tooltipRef}
          className="tooltip"
          role="tooltip"
          id={tooltipId}
          style={tooltipStyle}
        >
          {label}
        </span>
      ) : null}
    </span>
  );
}
