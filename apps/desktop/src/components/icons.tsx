interface IconProps {
  className?: string;
}

export function MenuIcon({ className }: IconProps) {
  return (
    <svg className={className} viewBox="0 0 16 16" width="16" height="16" aria-hidden="true">
      <path
        d="M2 4h12M2 8h12M2 12h12"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinecap="round"
      />
    </svg>
  );
}

/** Shared chrome stroke width for the Settings gear. */
export const SETTINGS_ICON_STROKE_WIDTH = 1.5;
/** Minimum clear space between stroked ink and each viewBox edge. */
export const SETTINGS_ICON_EDGE_PADDING = 1.5;

export function SettingsIcon({ className }: IconProps) {
  return (
    <svg className={className} viewBox="0 0 16 16" width="16" height="16" aria-hidden="true">
      <path
        d="M8.26 4.71L9.02 3.77L10.27 4.29L10.14 5.49L10.51 5.86L11.71 5.73L12.23 6.98L11.29 7.74L11.29 8.26L12.23 9.02L11.71 10.27L10.51 10.14L10.14 10.51L10.27 11.71L9.02 12.23L8.26 11.29L7.74 11.29L6.98 12.23L5.73 11.71L5.86 10.51L5.49 10.14L4.29 10.27L3.77 9.02L4.71 8.26L4.71 7.74L3.77 6.98L4.29 5.73L5.49 5.86L5.86 5.49L5.73 4.29L6.98 3.77L7.74 4.71Z"
        fill="none"
        stroke="currentColor"
        strokeWidth={SETTINGS_ICON_STROKE_WIDTH}
        strokeLinejoin="round"
        strokeLinecap="round"
      />
      <circle
        cx="8"
        cy="8"
        r="1.55"
        fill="none"
        stroke="currentColor"
        strokeWidth={SETTINGS_ICON_STROKE_WIDTH}
      />
    </svg>
  );
}

/** Conservative stroked bounds for the Settings gear inside its 16×16 viewBox. */
export function settingsIconStrokedBounds(svg: Element): { min: number; max: number } {
  const numbers: number[] = [];
  for (const path of svg.querySelectorAll("path")) {
    const data = path.getAttribute("d") ?? "";
    for (const match of data.matchAll(/-?\d*\.?\d+/g)) {
      numbers.push(Number(match[0]));
    }
  }
  for (const circle of svg.querySelectorAll("circle")) {
    const cx = Number(circle.getAttribute("cx"));
    const cy = Number(circle.getAttribute("cy"));
    const radius = Number(circle.getAttribute("r"));
    numbers.push(cx - radius, cx + radius, cy - radius, cy + radius);
  }

  const strokeWidth = Number(
    svg.querySelector("path")?.getAttribute("stroke-width") ?? SETTINGS_ICON_STROKE_WIDTH,
  );
  // Half-stroke plus an equal allowance for round joins at tooth tips.
  const outset = strokeWidth;
  return {
    min: Math.min(...numbers) - outset,
    max: Math.max(...numbers) + outset,
  };
}

export function JumpToLatestIcon({ className }: IconProps) {
  return (
    <svg className={className} viewBox="0 0 16 16" width="16" height="16" aria-hidden="true">
      <path
        d="M8 3.25v8.5M4.75 8.5 8 11.75 11.25 8.5"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

export function PlusIcon({ className }: IconProps) {
  return (
    <svg className={className} viewBox="0 0 16 16" width="16" height="16" aria-hidden="true">
      <path
        d="M8 3v10M3 8h10"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinecap="round"
      />
    </svg>
  );
}

export function ProjectsIcon({ className }: IconProps) {
  return (
    <svg className={className} viewBox="0 0 16 16" width="16" height="16" aria-hidden="true">
      <path
        d="M2.5 4.5h4l1 1.5h6v7h-11v-8.5Z"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.2"
        strokeLinejoin="round"
      />
    </svg>
  );
}
