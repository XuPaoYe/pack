type AntigravityIconProps = {
  className?: string;
  size?: number;
};

export function AntigravityIcon({ className, size = 20 }: AntigravityIconProps) {
  return (
    <svg className={className} width={size} height={size} viewBox="0 0 24 24" aria-hidden="true" fill="none">
      <path
        d="M12 2.5c-.42 0-.81.22-1.03.58l-3.7 6.21a1.2 1.2 0 0 0 .42 1.66 1.2 1.2 0 0 0 1.66-.4l1.45-2.42v12.66a1.2 1.2 0 0 0 2.4 0V8.13l1.45 2.42a1.2 1.2 0 0 0 1.66.4 1.2 1.2 0 0 0 .42-1.66l-3.7-6.21A1.2 1.2 0 0 0 12 2.5Z"
        fill="currentColor"
      />
      <circle cx="6.4" cy="14.6" r="1.05" fill="currentColor" />
      <circle cx="17.6" cy="14.6" r="1.05" fill="currentColor" />
      <circle cx="5.1" cy="19.1" r="0.85" fill="currentColor" opacity="0.7" />
      <circle cx="18.9" cy="19.1" r="0.85" fill="currentColor" opacity="0.7" />
    </svg>
  );
}
