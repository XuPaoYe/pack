type WindsurfIconProps = {
  className?: string;
  size?: number;
};

export function WindsurfIcon({ className, size = 20 }: WindsurfIconProps) {
  return (
    <svg className={className} width={size} height={size} viewBox="0 0 24 24" aria-hidden="true" fill="none">
      <path
        d="M3 7.5h11.5a3.5 3.5 0 1 0-3.4-4.3"
        stroke="currentColor"
        strokeWidth="1.7"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <path
        d="M3 12h15a3 3 0 1 1-3 3"
        stroke="currentColor"
        strokeWidth="1.7"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <path
        d="M3 16.5h9.5"
        stroke="currentColor"
        strokeWidth="1.7"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}
