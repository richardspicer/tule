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

export function SettingsIcon({ className }: IconProps) {
  return (
    <svg className={className} viewBox="0 0 16 16" width="16" height="16" aria-hidden="true">
      <path
        d="M6.5 2.5h3l.4 1.4a4.5 4.5 0 0 1 1.1.6l1.4-.4 1.5 1.5-.4 1.4c.2.3.4.7.6 1.1l1.4.4v3l-1.4.4a4.5 4.5 0 0 1-.6 1.1l.4 1.4-1.5 1.5-1.4-.4a4.5 4.5 0 0 1-1.1.6l-.4 1.4h-3l-.4-1.4a4.5 4.5 0 0 1-1.1-.6l-1.4.4-1.5-1.5.4-1.4a4.5 4.5 0 0 1-.6-1.1L2.5 9.5v-3l1.4-.4c.2-.4.4-.8.6-1.1l-.4-1.4 1.5-1.5 1.4.4c.3-.2.7-.4 1.1-.6L6.5 2.5Z"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.2"
        strokeLinejoin="round"
      />
      <circle cx="8" cy="8" r="2" fill="none" stroke="currentColor" strokeWidth="1.2" />
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
