import React from "react";
import { AbsoluteFill, Img, Sequence, interpolate, staticFile, useCurrentFrame } from "remotion";
import { font, mono } from "./theme";
import { Typewriter, useIn } from "./ui";

// « Clear » — 1920×1080, 30 fps, noir sur blanc, surligneur jaune. Rapide sur
// les listes, lent sur les quatre piliers et sur la commande finale. Aucun
// concurrent nommé. Données inventées sauf les mesures (docs/BENCHMARKS.md).

const FPS = 30;
const S = (sec: number) => Math.round(sec * FPS);
export const CLEAR_FRAMES = S(30);

/** Palette claire. */
const L = { bg: "#ffffff", ink: "#0f1115", muted: "#6b7280", line: "#e5e7eb", panel: "#f4f4f5", yellow: "#f5c518", green: "#15803d", red: "#b91c1c" };

const Page: React.FC<{ children: React.ReactNode }> = ({ children }) => (
  <AbsoluteFill style={{ background: L.bg, color: L.ink, fontFamily: font, overflow: "hidden" }}>{children}</AbsoluteFill>
);

/** Coupe nette avec un fondu très court : sobre. */
const Cut: React.FC<{ from: number; dur: number; children: React.ReactNode }> = ({ from, dur, children }) => {
  const Inner: React.FC = () => {
    const frame = useCurrentFrame();
    const o = interpolate(frame, [0, 4, dur - 5, dur], [0, 1, 1, 0], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
    return <AbsoluteFill style={{ opacity: o }}>{children}</AbsoluteFill>;
  };
  return <Sequence from={from} durationInFrames={dur}><Inner /></Sequence>;
};

const Center: React.FC<{ children: React.ReactNode; gap?: number }> = ({ children, gap = 30 }) => (
  <AbsoluteFill style={{ padding: 120, display: "flex", flexDirection: "column", justifyContent: "center", alignItems: "center", gap, textAlign: "center" }}>{children}</AbsoluteFill>
);

const In: React.FC<{ delay?: number; children: React.ReactNode; style?: React.CSSProperties; from?: number; snap?: boolean }> = ({ delay = 0, children, style, from = 26, snap }) => {
  const p = useIn(delay, snap ? 14 : 200);
  return <div style={{ opacity: Math.min(1, p * 1.3), transform: `translateY(${(1 - p) * from}px)`, ...style }}>{children}</div>;
};

const Big: React.FC<{ children: React.ReactNode; size?: number; color?: string }> = ({ children, size = 96, color = L.ink }) => (
  <div style={{ fontSize: size, fontWeight: 800, letterSpacing: -size * 0.03, lineHeight: 1.05, color }}>{children}</div>
);

/** Surligneur : texte noir sur bande jaune, comme un marqueur. */
const Hi: React.FC<{ children: React.ReactNode }> = ({ children }) => (
  <span style={{ background: L.yellow, padding: "0 14px", borderRadius: 6, boxDecorationBreak: "clone", WebkitBoxDecorationBreak: "clone" }}>{children}</span>
);

const Small: React.FC<{ children: React.ReactNode; color?: string }> = ({ children, color = L.muted }) => (
  <div style={{ fontSize: 38, color, lineHeight: 1.3, maxWidth: 1500 }}>{children}</div>
);

const Mark: React.FC<{ size?: number }> = ({ size = 96 }) => (
  <Img src={staticFile("logos/nd-two-rs.svg")} style={{ width: size, height: size, display: "block" }} />
);

const Brand: React.FC<{ size?: number }> = ({ size = 120 }) => (
  <div style={{ display: "flex", alignItems: "center", gap: size * 0.2 }}>
    <Mark size={size * 0.95} />
    <div style={{ fontSize: size, fontWeight: 800, letterSpacing: -size * 0.045, lineHeight: 1 }}>notify<span style={{ color: L.yellow, textShadow: `0 0 0 ${L.yellow}` }}>d</span></div>
  </div>
);

// ── Piliers ──────────────────────────────────────────────────────────────────
const PILLARS: Array<[string, string]> = [
  ["Self-hosted.", "your server, your providers, your data"],
  ["Agent-native.", "operated over MCP, no dashboard"],
  ["Open source.", "MIT, no per-notification bill"],
  ["Rust.", "one 12 MB binary, 13 MB of RAM"],
];

// ── Ce qui change ────────────────────────────────────────────────────────────
const ROWS: Array<[string, string]> = [
  ["MongoDB + Redis + S3", "the Postgres you already run"],
  ["7 containers", "1 binary · 12 MB"],
  ["a dashboard to log into", "an API, operated by your agent"],
  ["billed per notification", "€0 · pay your providers, nothing else"],
  ["a 429 fails the send", "paused · nothing lost · urgent first"],
  ["a platform to babysit", "13 MB of RAM, next to your app"],
];

const Row: React.FC<{ left: string; right: string; delay: number }> = ({ left, right, delay }) => {
  const l = useIn(delay, 14);
  const r = useIn(delay + 5, 14);
  return (
    <div style={{ display: "grid", gridTemplateColumns: "1fr 1.6fr", gap: 50, alignItems: "center" }}>
      <div style={{ opacity: l, transform: `translateX(${(1 - l) * -40}px)`, textAlign: "right", fontSize: 44, fontWeight: 700, color: L.muted, textDecoration: r > 0.5 ? "line-through" : "none", textDecorationColor: L.red, textDecorationThickness: 5 }}>{left}</div>
      <div style={{ opacity: r, transform: `translateX(${(1 - r) * 40}px)`, fontSize: 46, fontWeight: 800, color: L.ink, whiteSpace: "nowrap" }}>{right}</div>
    </div>
  );
};

// ── L'agent ──────────────────────────────────────────────────────────────────
const Bubble: React.FC<{ who: "you" | "agent"; children: React.ReactNode; delay: number }> = ({ who, children, delay }) => {
  const p = useIn(delay);
  const you = who === "you";
  return (
    <div style={{ opacity: p, transform: `translateY(${(1 - p) * 24}px)`, alignSelf: you ? "flex-end" : "flex-start", maxWidth: 1250, background: you ? L.yellow : L.panel, color: L.ink, border: `2px solid ${you ? L.yellow : L.line}`, borderRadius: 24, padding: "24px 34px", fontSize: 40, lineHeight: 1.35 }}>
      {children}
    </div>
  );
};

const Tool: React.FC<{ delay: number; call: string; result: string }> = ({ delay, call, result }) => {
  const p = useIn(delay);
  const r = useIn(delay + 12);
  return (
    <div style={{ opacity: p, alignSelf: "flex-start", fontFamily: mono, fontSize: 30, color: L.muted, paddingLeft: 24, borderLeft: `4px solid ${L.line}`, lineHeight: 1.5 }}>
      <div><span style={{ color: L.ink, fontWeight: 700 }}>▸ notifyd.</span>{call}</div>
      <div style={{ opacity: r, color: L.green }}>{result}</div>
    </div>
  );
};

// ── Terminal clair ───────────────────────────────────────────────────────────
const Term: React.FC<{ children: React.ReactNode; width?: number }> = ({ children, width = 1500 }) => (
  <div style={{ width, background: L.ink, borderRadius: 18, boxShadow: "0 30px 80px rgba(15,17,21,.18)", overflow: "hidden", textAlign: "left" }}>
    <div style={{ display: "flex", alignItems: "center", gap: 10, padding: "14px 20px", background: "#1b1f27" }}>
      <span style={{ width: 14, height: 14, borderRadius: 7, background: "#ff5f57" }} />
      <span style={{ width: 14, height: 14, borderRadius: 7, background: "#febc2e" }} />
      <span style={{ width: 14, height: 14, borderRadius: 7, background: "#28c840" }} />
      <span style={{ marginLeft: 14, fontFamily: mono, fontSize: 20, color: "#8a8f9c" }}>terminal</span>
    </div>
    <pre style={{ margin: 0, padding: "28px 34px", fontFamily: mono, fontSize: 38, lineHeight: 1.55, whiteSpace: "pre-wrap", color: "#e8e6e1" }}>{children}</pre>
  </div>
);

export const Clear: React.FC = () => (
  <Page>
    {/* 0 · Pour qui, vite */}
    <Cut from={0} dur={S(2)}>
      <Center gap={30}>
        <In snap><Big size={120}>You ship a product.</Big></In>
        <div style={{ display: "flex", gap: 22 }}>
          {["reset links", "shipped parcels", "2FA codes", "a red badge"].map((t, i) => (
            <In key={t} delay={8 + i * 4} snap><span style={{ display: "inline-block", border: `2px solid ${L.line}`, borderRadius: 999, padding: "12px 28px", fontSize: 34, background: L.panel }}>{t}</span></In>
          ))}
        </div>
      </Center>
    </Cut>
    <Cut from={S(2)} dur={S(2.2)}>
      <Center gap={26}>
        <In snap><Big size={104}>It has to reach people.</Big></In>
        <In delay={12}><Big size={72}><Hi>Without running a notification platform.</Hi></Big></In>
      </Center>
    </Cut>

    {/* 1 · Quatre piliers : on ralentit */}
    <Cut from={S(4.2)} dur={S(6.4)}>
      <AbsoluteFill style={{ padding: "0 200px", display: "flex", flexDirection: "column", justifyContent: "center", gap: 34 }}>
        {PILLARS.map(([big, small], i) => (
          <In key={big} delay={i * S(1.05)} snap from={40}>
            <div style={{ display: "flex", alignItems: "center", gap: 40 }}>
              <div style={{ fontSize: 108, fontWeight: 800, letterSpacing: -4, lineHeight: 1.1, minWidth: 760 }}>{i === 3 ? <Hi>{big}</Hi> : big}</div>
              <div style={{ fontSize: 38, color: L.muted, whiteSpace: "nowrap" }}>{small}</div>
            </div>
          </In>
        ))}
      </AbsoluteFill>
    </Cut>

    {/* 2 · Ce qui change : vite */}
    <Cut from={S(10.6)} dur={S(7.6)}>
      <AbsoluteFill style={{ padding: "110px 120px", display: "flex", flexDirection: "column", justifyContent: "center", gap: 30 }}>
        <div style={{ display: "grid", gridTemplateColumns: "1fr 1.6fr", gap: 50, fontFamily: mono, fontSize: 26, color: L.muted, letterSpacing: 2 }}>
          <In style={{ textAlign: "right" }}>THE USUAL WAY</In>
          <In delay={3} style={{ color: L.ink, fontWeight: 700 }}>NOTIFYD</In>
        </div>
        {ROWS.map(([l, r], i) => <Row key={l} left={l} right={r} delay={6 + i * S(0.95)} />)}
      </AbsoluteFill>
    </Cut>

    {/* 3 · L'agent : on ralentit */}
    <Cut from={S(18.2)} dur={S(5.4)}>
      <AbsoluteFill style={{ padding: "150px 170px 80px", display: "flex", flexDirection: "column", gap: 24 }}>
        <In><div style={{ fontFamily: mono, fontSize: 26, color: L.muted }}>your agent · notifyd MCP connected</div></In>
        <Bubble who="you" delay={6}>Anything wrong with notifications today?</Bubble>
        <Tool delay={S(0.9)} call={`digest()`} result={`0 findings · 12 400 sent · 0 failed · bounce 0.2 %`} />
        <Bubble who="agent" delay={S(1.9)}>Nothing. 12 400 sent, no failures. The provider slowed us for 47 s at 11:06; nothing was lost.</Bubble>
        <Sequence from={S(3.4)} layout="none"><In><Small color={L.ink}>No dashboard. <Hi>Your agent operates it, natively.</Hi></Small></In></Sequence>
      </AbsoluteFill>
    </Cut>

    {/* 4 · Pour qui, et la commande */}
    <Cut from={S(23.6)} dur={S(6.4)}>
      <Center gap={34}>
        <In snap><Big size={50}>For the team that already runs Postgres and works with an agent,<br />and would rather ship than babysit.</Big></In>
        <In delay={14}>
          <Term>
            <span style={{ color: "#8a8f9c" }}>$ </span><Typewriter start={14} cps={75} text="git clone https://github.com/rmzlb/notifyd && cd notifyd" />{"\n"}
            <Sequence from={S(1.5)} layout="none"><span><span style={{ color: "#8a8f9c" }}>$ </span><Typewriter start={0} cps={50} text="docker compose up -d" /></span></Sequence>{"\n"}
            <Sequence from={S(2.4)} layout="none"><span style={{ color: "#3fb950" }}>→ notifyd listening on :3400 · 44 MB image · Postgres only</span></Sequence>
          </Term>
        </In>
        <Sequence from={S(3.4)} layout="none">
          <In><div style={{ display: "flex", alignItems: "center", gap: 60 }}>
            <Brand size={96} />
            <Small>MIT · <span style={{ fontFamily: mono, color: L.ink }}>github.com/rmzlb/notifyd</span> · in production for three companies</Small>
          </div></In>
        </Sequence>
      </Center>
    </Cut>
  </Page>
);
