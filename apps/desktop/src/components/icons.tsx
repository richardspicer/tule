interface IconProps {
  className?: string;
}

export function MenuIcon({ className }: IconProps) {
  return (
    <svg className={className} viewBox="0 0 20 20" width="20" height="20" aria-hidden="true">
      <path
        d="M2.5 5h15M2.5 10h15M2.5 15h15"
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
    <svg className={className} viewBox="0 0 20 20" width="20" height="20" aria-hidden="true">
      <path
        d="M10.33 5.89L11.28 4.71L12.84 5.36L12.68 6.86L13.14 7.33L14.64 7.16L15.29 8.73L14.11 9.67L14.11 10.33L15.29 11.28L14.64 12.84L13.14 12.68L12.68 13.14L12.84 14.64L11.28 15.29L10.33 14.11L9.67 14.11L8.73 15.29L7.16 14.64L7.33 13.14L6.86 12.68L5.36 12.84L4.71 11.28L5.89 10.33L5.89 9.67L4.71 8.73L5.36 7.16L6.86 7.33L7.33 6.86L7.16 5.36L8.73 4.71L9.67 5.89Z"
        fill="none"
        stroke="currentColor"
        strokeWidth={SETTINGS_ICON_STROKE_WIDTH}
        strokeLinejoin="round"
        strokeLinecap="round"
      />
      <circle
        cx="10"
        cy="10"
        r="1.94"
        fill="none"
        stroke="currentColor"
        strokeWidth={SETTINGS_ICON_STROKE_WIDTH}
      />
    </svg>
  );
}

/** Conservative stroked bounds for the Settings gear inside its 20×20 viewBox. */
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
    <svg className={className} viewBox="0 0 20 20" width="20" height="20" aria-hidden="true">
      <path
        d="M10 4.06v10.63M5.94 10.63 10 14.69 14.06 10.63"
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
    <svg className={className} viewBox="0 0 20 20" width="20" height="20" aria-hidden="true">
      <path
        d="M10 3.75v12.5M3.75 10h12.5"
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
    <svg className={className} viewBox="0 0 20 20" width="20" height="20" aria-hidden="true">
      <path
        d="M3.13 5.63h5l1.25 1.88h7.5v8.75h-13.75v-10.63Z"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.2"
        strokeLinejoin="round"
      />
    </svg>
  );
}

export function SendIcon({ className }: IconProps) {
  return (
    <svg className={className} viewBox="0 0 20 20" width="20" height="20" aria-hidden="true">
      <path
        d="M4.38 10.63 15.63 5 10 15.63 8.75 11.88 4.38 10.63Z"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinejoin="round"
        strokeLinecap="round"
      />
    </svg>
  );
}

export function AttachFileIcon({ className }: IconProps) {
  return (
    <svg className={className} viewBox="0 0 20 20" width="20" height="20" aria-hidden="true">
      <path
        d="M6.88 3.13h5l2.5 2.5v10.63a1 1 0 0 1-1 1h-7.5a1 1 0 0 1-1-1v-12.5a1 1 0 0 1 1-1Z"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinejoin="round"
      />
      <path
        d="M11.88 3.13V6.25h3.13"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinejoin="round"
        strokeLinecap="round"
      />
    </svg>
  );
}

export function AttachFolderIcon({ className }: IconProps) {
  return (
    <svg className={className} viewBox="0 0 20 20" width="20" height="20" aria-hidden="true">
      <path
        d="M3.13 5.94h5.31L10 7.81h6.88v8.13h-13.75V5.94Z"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinejoin="round"
        strokeLinecap="round"
      />
    </svg>
  );
}

export function RemoveIcon({ className }: IconProps) {
  return (
    <svg className={className} viewBox="0 0 20 20" width="20" height="20" aria-hidden="true">
      <path
        d="M5.94 5.94 14.06 14.06M14.06 5.94 5.94 14.06"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinecap="round"
      />
    </svg>
  );
}
