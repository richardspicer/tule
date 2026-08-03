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
        d="M6.35 1.75h3.3l.35 1.7c.42.14.8.34 1.14.6l1.55-.55 1.65 1.65-.55 1.55c.26.34.46.72.6 1.14l1.7.35v3.3l-1.7.35c-.14.42-.34.8-.6 1.14l.55 1.55-1.65 1.65-1.55-.55a4.6 4.6 0 0 1-1.14.6l-.35 1.7h-3.3l-.35-1.7a4.6 4.6 0 0 1-1.14-.6l-1.55.55-1.65-1.65.55-1.55a4.6 4.6 0 0 1-.6-1.14l-1.7-.35v-3.3l1.7-.35c.14-.42.34-.8.6-1.14l-.55-1.55L4.01 3.5l1.55.55c.34-.26.72-.46 1.14-.6l.35-1.7Z"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinejoin="round"
      />
      <circle cx="8" cy="8" r="2" fill="none" stroke="currentColor" strokeWidth="1.5" />
    </svg>
  );
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
