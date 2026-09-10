import React from "react";
import { interpolate, spring, useCurrentFrame, useVideoConfig } from "remotion";
import { C, font, mono } from "./theme";

export const Frame: React.FC<{ children: React.ReactNode }> = ({ children }) => (
  <div style={{ position: "absolute", inset: 0, background: C.bg, color: C.text, fontFamily: font, overflow: "hidden" }}>{children}</div>
);

export const useIn = (delay = 0, damping = 200) => {
  const frame = useCurrentFrame(); const { fps } = useVideoConfig();
  return spring({ frame: frame - delay, fps, config: { damping, stiffness: 120, mass: 0.8 } });
};

export const Appear: React.FC<{ delay?: number; children: React.ReactNode; style?: React.CSSProperties; from?: number }> = ({ delay = 0, children, style, from = 24 }) => {
  const p = useIn(delay);
  return <div style={{ opacity: p, transform: `translateY(${(1 - p) * from}px)`, ...style }}>{children}</div>;
};

export const FadeOut: React.FC<{ at: number; children: React.ReactNode }> = ({ at, children }) => {
  const frame = useCurrentFrame();
  const o = interpolate(frame, [at, at + 12], [1, 0], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
  return <div style={{ position: "absolute", inset: 0, opacity: o }}>{children}</div>;
};

export const Logo: React.FC<{ size?: number }> = ({ size = 64 }) => (
  <svg viewBox="0 0 80 80" width={size} height={size}>
    <rect width="80" height="80" rx="18" fill="#0f1115" stroke={C.line} strokeWidth="2" />
    <path d="M15 60 V41 C15 30 33 30 33 41 V60" fill="none" stroke={C.text} strokeWidth="9" strokeLinecap="round" strokeLinejoin="round" />
    <circle cx="54.5" cy="51" r="9" fill="none" stroke={C.yellow} strokeWidth="9" />
    <path d="M63.5 60 V19" fill="none" stroke={C.yellow} strokeWidth="9" strokeLinecap="round" />
  </svg>
);

export const Title: React.FC<{ children: React.ReactNode; small?: boolean }> = ({ children, small }) => (
  <div style={{ fontSize: small ? 34 : 52, fontWeight: 800, letterSpacing: -1.5, lineHeight: 1.1 }}>{children}</div>
);

export const Chip: React.FC<{ children: React.ReactNode; color?: string; bg?: string }> = ({ children, color = C.text, bg = C.panel }) => (
  <span style={{ display: "inline-block", border: `2px solid ${C.line}`, borderRadius: 10, padding: "6px 14px", fontSize: 22, color, background: bg }}>{children}</span>
);

export const Code: React.FC<{ children: React.ReactNode; size?: number; style?: React.CSSProperties }> = ({ children, size = 22, style }) => (
  <pre style={{ margin: 0, fontFamily: mono, fontSize: size, lineHeight: 1.45, background: C.panel, border: `2px solid ${C.line}`, borderRadius: 14, padding: "18px 22px", color: C.text, whiteSpace: "pre-wrap", ...style }}>{children}</pre>
);

export const Typewriter: React.FC<{ text: string; start: number; cps?: number; style?: React.CSSProperties }> = ({ text, start, cps = 40, style }) => {
  const frame = useCurrentFrame(); const { fps } = useVideoConfig();
  const n = Math.max(0, Math.floor(((frame - start) / fps) * cps));
  return <span style={style}>{text.slice(0, n)}{n < text.length && frame >= start ? "▍" : ""}</span>;
};

export const Y: React.FC<{ children: React.ReactNode }> = ({ children }) => <span style={{ color: C.yellow }}>{children}</span>;
export const M: React.FC<{ children: React.ReactNode }> = ({ children }) => <span style={{ color: C.muted }}>{children}</span>;
export const G: React.FC<{ children: React.ReactNode }> = ({ children }) => <span style={{ color: C.green }}>{children}</span>;
export const R: React.FC<{ children: React.ReactNode }> = ({ children }) => <span style={{ color: C.red }}>{children}</span>;
export const B: React.FC<{ children: React.ReactNode }> = ({ children }) => <span style={{ color: C.blue }}>{children}</span>;
