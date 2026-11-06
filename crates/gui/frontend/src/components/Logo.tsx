/**
 * Cerebro logo mark — brain silhouette (organic left hemisphere, circuit
 * trace on the right) in the app's indigo gradient.
 *
 * Geometry mirrors `crates/gui/icons/icon.svg`, the design source for the
 * generated app icons (`tauri icon`). `tile` renders the full app-icon
 * squircle; otherwise just the mark on transparency.
 */
export default function Logo({
  size = 20,
  tile = false,
}: {
  size?: number;
  tile?: boolean;
}) {
  const ids = { mark: tile ? "cm-t" : "cm-m" };
  return (
    <svg
      width={size}
      height={tile ? size : Math.round((size * 246) / 288)}
      viewBox={tile ? "0 0 512 512" : "112 126 288 246"}
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      aria-label="Cerebro"
      role="img"
    >
      <defs>
        <linearGradient
          id={ids.mark}
          x1="150"
          y1="140"
          x2="380"
          y2="370"
          gradientUnits="userSpaceOnUse"
        >
          <stop offset="0" stopColor="#b0baff" />
          <stop offset="0.55" stopColor="#8a90fa" />
          <stop offset="1" stopColor="#6d6ff5" />
        </linearGradient>
        {tile && (
          <>
            <linearGradient
              id="cerebro-bg"
              x1="256"
              y1="64"
              x2="256"
              y2="448"
              gradientUnits="userSpaceOnUse"
            >
              <stop offset="0" stopColor="#1a1a2e" />
              <stop offset="1" stopColor="#0b0b12" />
            </linearGradient>
            <radialGradient
              id="cerebro-ambient"
              cx="256"
              cy="244"
              r="195"
              gradientUnits="userSpaceOnUse"
            >
              <stop offset="0" stopColor="#6366f1" stopOpacity="0.22" />
              <stop offset="1" stopColor="#6366f1" stopOpacity="0" />
            </radialGradient>
            <filter id="cerebro-glow" x="-40%" y="-40%" width="180%" height="180%">
              <feGaussianBlur stdDeviation="7" />
            </filter>
          </>
        )}
      </defs>

      {tile && (
        <>
          <rect x="32" y="32" width="448" height="448" rx="100" fill="url(#cerebro-bg)" />
          <rect
            x="34"
            y="34"
            width="444"
            height="444"
            rx="98"
            fill="none"
            stroke="#ffffff"
            strokeOpacity="0.07"
            strokeWidth="3"
          />
          <rect x="32" y="32" width="448" height="448" rx="100" fill="url(#cerebro-ambient)" />
        </>
      )}

      <g
        transform="translate(256,244) scale(1.1) translate(-256,-252)"
        stroke={`url(#${ids.mark})`}
        strokeWidth="14"
        strokeLinecap="round"
        strokeLinejoin="round"
        fill="none"
      >
        <path d="M256,148 C180,148 128,196 128,258 C128,314 170,350 224,352 C240,353 252,351 256,348 C260,351 272,353 288,352 C342,350 384,314 384,258 C384,196 332,148 256,148 Z" />
        <path d="M256,160 L256,204" strokeOpacity="0.78" strokeWidth="11" />
        <path d="M198,212 C178,234 190,258 170,284" strokeOpacity="0.85" />
        <path d="M316,208 L316,244 L350,278" />
      </g>
      <g transform="translate(256,244) scale(1.1) translate(-256,-252)">
        <circle cx="316" cy="208" r="9.5" fill="#b0baff" />
        <circle cx="350" cy="278" r="12" fill="#8a90fa" />
      </g>
    </svg>
  );
}
