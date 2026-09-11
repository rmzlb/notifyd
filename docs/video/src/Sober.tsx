import React from "react";
import { AbsoluteFill, Img, Sequence, interpolate, staticFile, useCurrentFrame } from "remotion";
import { loadFont } from "@remotion/fonts";

// Deux films sobres, grammaire « produit à l'écran » : une carte titre en
// serif, un vrai terminal (sorties capturées sur une instance réelle le
// 10 septembre 2026, faux Resend qui répond 429 une fois), une légende d'une
// ligne par plan, une carte de fin avec la commande. Aucune typographie
// cinétique, aucun balayage : fondus courts, glissements de 8 px.

loadFont({ family: "Source Serif 4", url: staticFile("fonts/SourceSerif4.ttf") });
loadFont({ family: "JetBrains Mono", url: staticFile("fonts/JetBrainsMono.ttf") });
loadFont({ family: "Inter", url: staticFile("fonts/Inter.ttf") });

const SERIF = '"Source Serif 4", Georgia, serif';
const MONO = '"JetBrains Mono", ui-monospace, Menlo, monospace';
const SANS = 'Inter, ui-sans-serif, system-ui, sans-serif';

const FPS = 30;
const S = (sec: number) => Math.round(sec * FPS);

const D = { bg: "#141414", panel: "#1b1b1b", line: "#2a2a2a", text: "#ebe8e3", muted: "#8b8b8b", yellow: "#f5c518", green: "#7bd88f", amber: "#f2b134", red: "#f07a67", blue: "#8ab4f8" };
const L = { bg: "#f6f3ee", panel: "#ffffff", line: "#ddd7cc", ink: "#1c1c1c", muted: "#6f6b64", yellow: "#f5c518", green: "#2f8f4e", amber: "#c98a12", red: "#c2503e" };

// ── Primitives ──────────────────────────────────────────────────────────────
const ease = (t: number) => 1 - Math.pow(1 - t, 3);

/** Apparition douce : opacité + 8 px de glissement, sans ressort. */
const Fade: React.FC<{ at?: number; dur?: number; children: React.ReactNode; style?: React.CSSProperties; dy?: number }> = ({ at = 0, dur = 12, children, style, dy = 8 }) => {
  const f = useCurrentFrame();
  const p = ease(interpolate(f, [at, at + dur], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" }));
  return <div style={{ opacity: p, transform: `translateY(${(1 - p) * dy}px)`, ...style }}>{children}</div>;
};

/** Plan : fondu d'entrée et de sortie courts, zoom lent de 1 à 1,015. */
const Shot: React.FC<{ from: number; dur: number; children: React.ReactNode; zoom?: boolean; xfade?: boolean }> = ({ from, dur, children, zoom = true, xfade = false }) => {
  // `xfade` : le plan démarre 8 images plus tôt, pendant le fondu de sortie du
  // précédent, ce qui remplace le passage au noir par un fondu enchaîné.
  const start = xfade && from > 0 ? from - 8 : from;
  const length = dur + (from - start);
  const Inner: React.FC = () => {
    const f = useCurrentFrame();
    const o = interpolate(f, [0, 8, length - 8, length], [0, 1, 1, 0], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
    const z = zoom ? interpolate(f, [0, length], [1, 1.015]) : 1;
    return <AbsoluteFill style={{ opacity: o, transform: `scale(${z})` }}>{children}</AbsoluteFill>;
  };
  return <Sequence from={start} durationInFrames={length}><Inner /></Sequence>;
};

const Mark: React.FC<{ size?: number }> = ({ size = 36 }) => (
  <Img src={staticFile("logos/nd-two-rs.svg")} style={{ width: size, height: size, display: "block" }} />
);

/** Légende serif sous l'écran, comme une phrase de documentation. */
const Caption: React.FC<{ children: React.ReactNode; at?: number; dark?: boolean }> = ({ children, at = 20, dark = true }) => (
  <Fade at={at} style={{ position: "absolute", left: 0, right: 0, bottom: 84, textAlign: "center", fontFamily: SERIF, fontSize: 40, lineHeight: 1.35, color: dark ? D.text : L.ink, padding: "0 240px" }}>
    {children}
  </Fade>
);

/** Fenêtre de terminal ; les enfants sont des lignes déjà mises en forme. */
const Terminal: React.FC<{ children: React.ReactNode; width?: number; title?: string; dark?: boolean; size?: number }> = ({ children, width = 1660, title = "Terminal — notifyd", dark = true, size = 22 }) => (
  <div style={{ width, background: dark ? D.panel : "#1b1b1b", border: `1px solid ${D.line}`, borderRadius: 14, boxShadow: "0 40px 120px rgba(0,0,0,.55)", overflow: "hidden" }}>
    <div style={{ display: "flex", alignItems: "center", gap: 8, padding: "12px 16px", borderBottom: `1px solid ${D.line}`, background: "#202020" }}>
      <span style={{ width: 12, height: 12, borderRadius: 6, background: "#3a3a3a" }} />
      <span style={{ width: 12, height: 12, borderRadius: 6, background: "#3a3a3a" }} />
      <span style={{ width: 12, height: 12, borderRadius: 6, background: "#3a3a3a" }} />
      <span style={{ marginLeft: "auto", marginRight: "auto", fontFamily: MONO, fontSize: 18, color: D.muted }}>{title}</span>
    </div>
    <pre style={{ margin: 0, padding: "26px 30px", fontFamily: MONO, fontSize: size, lineHeight: 1.6, color: D.text, whiteSpace: "pre-wrap", textAlign: "left", minHeight: 520 }}>{children}</pre>
  </div>
);

/** Une ligne de terminal qui apparaît au frame `at` ; `type` la tape. */
const Line: React.FC<{ at: number; children?: React.ReactNode; type?: string; cps?: number; color?: string }> = ({ at, children, type, cps = 55, color }) => {
  const f = useCurrentFrame();
  if (f < at) return null;
  if (type !== undefined) {
    const n = Math.min(type.length, Math.floor(((f - at) / FPS) * cps));
    const done = n >= type.length;
    return (
      <div style={{ color }}>
        <span style={{ color: D.muted }}>$ </span>
        {type.slice(0, n)}
        {!done && <span style={{ display: "inline-block", width: 11, height: 22, background: D.text, verticalAlign: "-4px", marginLeft: 2 }} />}
      </div>
    );
  }
  const p = ease(interpolate(f, [at, at + 6], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" }));
  return <div style={{ opacity: p, color }}>{children}</div>;
};

const K: React.FC<{ children: React.ReactNode }> = ({ children }) => <span style={{ color: D.blue }}>{children}</span>;
const V: React.FC<{ children: React.ReactNode }> = ({ children }) => <span style={{ color: D.green }}>{children}</span>;
const M: React.FC<{ children: React.ReactNode }> = ({ children }) => <span style={{ color: D.muted }}>{children}</span>;
const Y: React.FC<{ children: React.ReactNode }> = ({ children }) => <span style={{ color: D.yellow }}>{children}</span>;
const A: React.FC<{ children: React.ReactNode }> = ({ children }) => <span style={{ color: D.amber }}>{children}</span>;

const TitleCard: React.FC<{ title: string; sub: React.ReactNode; dark?: boolean }> = ({ title, sub, dark = true }) => (
  <AbsoluteFill style={{ padding: "0 200px", display: "flex", flexDirection: "column", justifyContent: "center", gap: 28 }}>
    <Fade><Mark size={44} /></Fade>
    <Fade at={6}><div style={{ fontFamily: SERIF, fontSize: 92, lineHeight: 1.1, color: dark ? D.text : L.ink, letterSpacing: -1 }}>{title}</div></Fade>
    <Fade at={14}><div style={{ fontFamily: MONO, fontSize: 28, color: dark ? D.muted : L.muted }}>{sub}</div></Fade>
  </AbsoluteFill>
);

const EndCard: React.FC<{ dark?: boolean }> = ({ dark = true }) => (
  <AbsoluteFill style={{ display: "flex", flexDirection: "column", justifyContent: "center", alignItems: "center", gap: 30, textAlign: "center" }}>
    <Fade><Mark size={64} /></Fade>
    <Fade at={6}><div style={{ fontFamily: MONO, fontSize: 30, color: dark ? D.muted : L.muted }}>git clone https://github.com/rmzlb/notifyd && cd notifyd && docker compose up -d</div></Fade>
    <Fade at={12}><div style={{ fontFamily: SERIF, fontSize: 76, color: dark ? D.text : L.ink, letterSpacing: -0.5 }}>One binary for every notification.</div></Fade>
    <Fade at={20}><div style={{ fontFamily: SANS, fontSize: 24, color: dark ? D.muted : L.muted, letterSpacing: 3, textTransform: "uppercase" }}>notifyd · MIT · github.com/rmzlb/notifyd</div></Fade>
  </AbsoluteFill>
);

/** AbsoluteFill is a column flexbox: `alignItems` centres horizontally. */
const Screen: React.FC<{ children: React.ReactNode }> = ({ children }) => (
  <AbsoluteFill style={{ alignItems: "center", paddingTop: 90 }}>
    <div>{children}</div>
  </AbsoluteFill>
);

// ══════════════════════════════════════════════════════════════════════════════
// Film 1 · « The terminal » — sombre, sorties réelles.
// ══════════════════════════════════════════════════════════════════════════════
export const SOBER_TERMINAL_FRAMES = S(48.8);

export const SoberTerminal: React.FC = () => (
  <AbsoluteFill style={{ background: D.bg, color: D.text, fontFamily: SANS }}>
    <Shot from={0} dur={S(3.4)} zoom={false}>
      <TitleCard title="One binary for every notification." sub={<>docker compose up -d <span style={{ color: "#555" }}>· email, sms, push, in-app, telegram, slack, discord</span></>} />
    </Shot>

    {/* 1 · up */}
    <Shot from={S(3.4)} dur={S(6.2)}>
      <Screen>
        <Terminal>
          <Line at={0} type="docker compose up -d" />
          <Line at={S(1.1)}><M> Container notifyd-notifyd-db-1  Started</M></Line>
          <Line at={S(1.4)}><M> Container notifyd-notifyd-db-1  Healthy</M></Line>
          <Line at={S(1.7)}><M> Container notifyd-notifyd-1     Started</M></Line>
          <Line at={S(2.3)} type="curl localhost:3400/v1/health" />
          <Line at={S(3.4)}>{"{"}<K>"status"</K>: <V>"ok"</V>, <K>"db"</K>: <V>"ok"</V>, <K>"version"</K>: <V>"0.2.2"</V>, <K>"uptime_seconds"</K>: 2{"}"}</Line>
        </Terminal>
      </Screen>
      <Caption at={S(3.8)}>Two containers: notifyd and the Postgres you already run. Nothing else to install.</Caption>
    </Shot>

    {/* 2 · send */}
    <Shot from={S(9.6)} dur={S(8)}>
      <Screen>
        <Terminal>
          <Line at={0} type="curl -X POST localhost:3400/v1/send -H 'X-Api-Key: $KEY' -d '{" cps={70} />
          <Line at={S(1.3)}>{"  "}<K>"subscriber_id"</K>: <V>"cust_4821"</V>,</Line>
          <Line at={S(1.5)}>{"  "}<K>"channels"</K>: [<V>"email"</V>, <V>"in_app"</V>, <V>"telegram"</V>],</Line>
          <Line at={S(1.7)}>{"  "}<K>"subject"</K>: <V>"Your order shipped"</V>,</Line>
          <Line at={S(1.9)}>{"  "}<K>"body"</K>: <V>"Hi {"{{first_name}}"}, parcel FR-2041 is on its way."</V>,</Line>
          <Line at={S(2.1)}>{"  "}<K>"idempotency_key"</K>: <V>"order-2041-shipped"</V></Line>
          <Line at={S(2.3)}>{"}'"}</Line>
          <Line at={S(3.2)}>{"{"}<K>"success"</K>: true, <K>"channels"</K>: [<V>"email"</V>, <V>"in_app"</V>, <V>"telegram"</V>],</Line>
          <Line at={S(3.4)}>{" "}<K>"job_ids"</K>: [<V>"cc67293b-…"</V>, <V>"a1579543-…"</V>, <V>"d98e7597-…"</V>], <K>"skipped"</K>: []{"}"}</Line>
        </Terminal>
      </Screen>
      <Caption at={S(4.2)}>One call. The customer's email, inbox and Telegram, from the record you already keep.</Caption>
    </Shot>

    {/* 3 · campaign + 429 */}
    <Shot from={S(17.6)} dur={S(9)}>
      <Screen>
        <Terminal>
          <Line at={0} type={`curl -X POST localhost:3400/v1/batch -d '{"segment": {"has_email": true}, "channel": "email", "template": "september-news"}'`} cps={90} />
          <Line at={S(2.2)}>{"{"}<K>"jobs_created"</K>: 41, <K>"jobs_deduplicated"</K>: 0, <K>"subscribers"</K>: 41{"}"}</Line>
          <Line at={S(3.2)} type="notifyd digest" />
          <Line at={S(4)}><M># notifyd digest — last 1h</M></Line>
          <Line at={S(4.2)}><M>Instance: email resend · </M><A>paused lanes: email</A></Line>
          <Line at={S(4.6)}><A>warning</A> — Lane email is paused after a provider 429.</Line>
          <Line at={S(4.8)}><M>  Transient. If it repeats, lower the lane's rate or ask the provider for a higher limit.</M></Line>
          <Line at={S(5.4)}><M>Queue</M>  pending 41 · retry 1 · processing 0 — oldest waiting: bulk 5s</Line>
        </Terminal>
      </Screen>
      <Caption at={S(5.8)}>The provider says slow down. The lane pauses for exactly as long as asked. Nothing is dropped.</Caption>
    </Shot>

    {/* 4 · jobs after */}
    <Shot from={S(26.6)} dur={S(6.4)}>
      <Screen>
        <Terminal width={1800} size={21}>
          <Line at={0} type="notifyd jobs --since 1h" />
          <Line at={S(1)}><M>id                                    created              channel   status   provider   recipient          subject</M></Line>
          <Line at={S(1.2)}>9d61b19e-473e-4896-9c59-416c1fd3bdd8  2026-09-10T19:07:04  email     <V>sent</V>     resend     c***@example.com   September news</Line>
          <Line at={S(1.35)}>c6f9e265-38f8-46af-89bb-877bbcb2cdac  2026-09-10T19:07:04  email     <V>sent</V>     resend     c***@example.com   September news</Line>
          <Line at={S(1.5)}>f63773e7-7260-44a2-9152-86b8b890fe1a  2026-09-10T19:07:04  email     <V>sent</V>     resend     a***@example.com   September news</Line>
          <Line at={S(1.65)}>d98e7597-ae75-43ef-b9e1-d29c276aa694  2026-09-10T19:07:02  telegram  <V>sent</V>     telegram   552211             Your order shipped</Line>
          <Line at={S(1.8)}>a1579543-2b93-493e-afbb-de8490810d3d  2026-09-10T19:07:02  in_app    <V>sent</V>     postgres   cust_4821          Your order shipped</Line>
          <Line at={S(1.95)}>cc67293b-b685-42e4-812d-1257e1896019  2026-09-10T19:07:02  email     <V>sent</V>     resend     a***@example.com   Your order shipped</Line>
          <Line at={S(2.3)}><M>44 job(s)</M></Line>
        </Terminal>
      </Screen>
      <Caption at={S(2.8)}>Forty-seven seconds later, everything went out, urgent mail first. Every job keeps its provider and its history.</Caption>
    </Shot>

    {/* 5 · Telegram */}
    <Shot from={S(33)} dur={S(6.6)}>
      <AbsoluteFill style={{ alignItems: "center", paddingTop: 150 }}>
        <div>
          <Fade><div style={{ fontFamily: MONO, fontSize: 22, color: D.muted, marginBottom: 14 }}>Telegram · 19:07 · notifyd</div></Fade>
          <Fade at={6}>
            <div style={{ width: 1100, background: D.panel, border: `1px solid ${D.line}`, borderRadius: 18, padding: "26px 32px", fontFamily: SANS, fontSize: 30, lineHeight: 1.45, color: D.text, whiteSpace: "pre-wrap" }}>
              {"notifyd: 1 warning(s)\n\n! Lane `email` is paused after a provider 429.\n   → Transient. If it repeats, lower the lane's rate or ask the provider for a higher limit.\n\nqueue pending 41 · retry 1 · processing 0\nsent 2 · failed 0"}
            </div>
          </Fade>
          <Fade at={S(2.4)}><div style={{ marginTop: 26, fontFamily: MONO, fontSize: 22, color: D.muted }}>the same digest as <span style={{ color: D.text }}>GET /v1/admin/digest</span> and the MCP tool <span style={{ color: D.text }}>digest</span></div></Fade>
        </div>
      </AbsoluteFill>
      <Caption at={S(3.2)}>The digest reaches you where you are, and your agent reads the same thing over MCP.</Caption>
    </Shot>

    {/* 6 · measured */}
    <Shot from={S(39.6)} dur={S(4.6)} zoom={false}>
      <AbsoluteFill style={{ justifyContent: "center", alignItems: "center" }}>
        <div style={{ display: "grid", gridTemplateColumns: "300px 420px 420px", rowGap: 22, columnGap: 40, fontFamily: SANS, fontSize: 34, alignItems: "baseline" }}>
          <Fade at={0}><span /></Fade>
          <Fade at={0}><div style={{ fontFamily: MONO, fontSize: 24, color: D.muted, letterSpacing: 2 }}>NOVU 3.19</div></Fade>
          <Fade at={4}><div style={{ fontFamily: MONO, fontSize: 24, color: D.yellow, letterSpacing: 2 }}>NOTIFYD 0.2.2</div></Fade>
          {[
            ["containers", "6", "1"],
            ["images to pull", "1.4 GB", "44 MB"],
            ["memory, idle", "1.1 GB", "13 MB"],
          ].map(([k, a, b], i) => (
            <React.Fragment key={k}>
              <Fade at={10 + i * 8}><div style={{ color: D.muted }}>{k}</div></Fade>
              <Fade at={12 + i * 8}><div style={{ fontFamily: SERIF, fontSize: 56, color: D.muted }}>{a}</div></Fade>
              <Fade at={14 + i * 8}><div style={{ fontFamily: SERIF, fontSize: 56, color: D.text }}>{b}</div></Fade>
            </React.Fragment>
          ))}
        </div>
      </AbsoluteFill>
      <Caption at={S(1.6)}>Novu's own community docker-compose, idle, measured on the same machine with the same tool. Method and caveats in docs/BENCHMARKS.md.</Caption>
    </Shot>

    <Shot from={S(44.2)} dur={S(4.6)} zoom={false}>
      <EndCard />
    </Shot>
  </AbsoluteFill>
);

// ══════════════════════════════════════════════════════════════════════════════
// Film 2 · « The diagram » — clair, un schéma qui se construit.
// ══════════════════════════════════════════════════════════════════════════════
export const SOBER_DIAGRAM_FRAMES = S(38);

const Node: React.FC<{ x: number; y: number; w?: number; h?: number; at: number; label: string; sub?: string; accent?: string; mono?: boolean; dark?: boolean }> = ({ x, y, w = 260, h = 88, at, label, sub, accent, mono, dark }) => (
  <Fade at={at} style={{ position: "absolute", left: x, top: y, width: w, height: h }} dy={6}>
    <div style={{ width: "100%", height: "100%", background: dark ? D.panel : L.panel, border: `1.5px solid ${accent ?? (dark ? D.line : L.line)}`, borderRadius: 12, display: "flex", flexDirection: "column", justifyContent: "center", alignItems: "center", boxShadow: dark ? "0 8px 30px rgba(0,0,0,.35)" : "0 8px 30px rgba(28,28,28,.06)" }}>
      <div style={{ fontFamily: mono ? MONO : SANS, fontSize: dark ? 30 : 26, fontWeight: 600, color: dark ? D.text : L.ink }}>{label}</div>
      {sub && <div style={{ fontFamily: MONO, fontSize: dark ? 20 : 18, color: accent ?? (dark ? D.muted : L.muted), marginTop: 4 }}>{sub}</div>}
    </div>
  </Fade>
);

/** Trait horizontal qui se dessine de gauche à droite. */
const Wire: React.FC<{ x: number; y: number; w: number; at: number; color?: string; dashed?: boolean }> = ({ x, y, w, at, color = L.line, dashed }) => {
  const f = useCurrentFrame();
  const p = ease(interpolate(f, [at, at + 14], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" }));
  return <div style={{ position: "absolute", left: x, top: y, width: w * p, borderTop: `2px ${dashed ? "dashed" : "solid"} ${color}` }} />;
};

const CHANNELS = ["email", "sms", "whatsapp", "push", "in-app", "telegram", "slack", "discord"];

export const SoberDiagram: React.FC = () => {
  const f = useCurrentFrame();
  const incident = f >= S(14) && f < S(21.5);
  const resolved = f >= S(21.5);
  return (
    <AbsoluteFill style={{ background: L.bg, color: L.ink, fontFamily: SANS }}>
      <Shot from={0} dur={S(3.2)} zoom={false}>
        <TitleCard dark={false} title="A notification server your agent can run." sub={<>one Rust binary · Postgres only · MIT</>} />
      </Shot>

      <Shot from={S(3.2)} dur={S(30.8)} zoom={false}>
        <div style={{ position: "absolute", inset: 0, transform: "translateX(240px)" }}>
        {/* App → notifyd → channels */}
        <Node x={140} y={420} at={0} label="your app" sub="POST /v1/send" />
        <Wire x={400} y={464} w={160} at={8} />
        <Node x={560} y={380} w={360} h={170} at={14} label="notifyd" sub="one binary · 12 MB" accent={L.ink} />
        <Node x={610} y={580} w={260} h={64} at={22} label="Postgres" mono />
        <Wire x={920} y={464} w={140} at={30} />
        <Fade at={32} style={{ position: "absolute", left: 1059, top: 182, height: 646, borderLeft: `2px solid ${L.line}` }} dy={0}><span /></Fade>
        {CHANNELS.map((c, i) => (
          <React.Fragment key={c}>
            <Node x={1180} y={150 + i * 92} w={240} h={64} at={32 + i * 4} label={c} mono
              accent={c === "email" ? (incident ? L.amber : resolved ? L.green : undefined) : undefined}
              sub={c === "email" ? (incident ? "429 · paused 47 s" : resolved ? "41 sent" : undefined) : undefined}
            />
            <Wire x={1060} y={182 + i * 92} w={120} at={30 + i * 3} color={c === "email" && incident ? L.amber : L.line} />
          </React.Fragment>
        ))}
        </div>
        <Sequence from={0} durationInFrames={S(10.8)} layout="none">
          <Caption dark={false} at={S(4.4)}>One call in. Eight channels out. Priorities, retries and quiet hours belong to the server.</Caption>
        </Sequence>

        {/* Incident */}
        <Sequence from={S(10.8)} durationInFrames={S(10.7)} layout="none">
          <Fade at={0} style={{ position: "absolute", left: 800, top: 300, width: 360 }}>
            <div style={{ fontFamily: MONO, fontSize: 22, color: L.amber, textAlign: "center" }}>queue · 41 waiting · urgent first</div>
          </Fade>
          <Caption dark={false} at={S(1.6)}>A provider says slow down. The lane waits exactly as long as asked; a password reset still goes first.</Caption>
        </Sequence>
        <Sequence from={S(21.5)} durationInFrames={S(6.3)} layout="none">
          <Fade at={0} style={{ position: "absolute", left: 800, top: 300, width: 360 }}>
            <div style={{ fontFamily: MONO, fontSize: 22, color: L.green, textAlign: "center" }}>queue · 0 waiting · 41 sent</div>
          </Fade>
          {/* Agent */}
          <Node x={800} y={120} w={360} h={88} at={S(0.8)} label="your agent" sub="MCP · digest, jobs, retry, settings" />
          <Fade at={S(1)} style={{ position: "absolute", left: 979, top: 208, height: 172, borderLeft: `2px dashed ${L.ink}` }} dy={0}><span /></Fade>
          <Fade at={S(2.2)} style={{ position: "absolute", left: 380, top: 140, width: 380 }}>
            <div style={{ background: L.panel, border: `1.5px solid ${L.line}`, borderRadius: 14, padding: "16px 20px", fontFamily: SANS, fontSize: 21, lineHeight: 1.4, color: L.ink }}>
              <div style={{ fontFamily: MONO, fontSize: 16, color: L.muted, marginBottom: 6 }}>Telegram · 19:07 · notifyd</div>
              notifyd: 1 warning. Lane email paused after a provider 429. Transient; nothing lost.
            </div>
          </Fade>
          <Caption dark={false} at={S(2.8)}>No dashboard. Your agent operates it over MCP, and the digest lands in your chat.</Caption>
        </Sequence>

        {/* Measured */}
        <Sequence from={S(27.8)} durationInFrames={S(6.2)} layout="none">
          <Fade at={0} style={{ position: "absolute", left: 380, top: 690 }}>
            <div style={{ display: "grid", gridTemplateColumns: "220px 240px 240px", columnGap: 30, rowGap: 10, fontFamily: SANS, fontSize: 26, alignItems: "baseline", background: L.panel, border: `1.5px solid ${L.line}`, borderRadius: 14, padding: "22px 34px" }}>
              <span />
              <div style={{ fontFamily: MONO, fontSize: 18, color: L.muted, letterSpacing: 2 }}>NOVU 3.19</div>
              <div style={{ fontFamily: MONO, fontSize: 18, color: L.ink, letterSpacing: 2 }}>NOTIFYD</div>
              <div style={{ color: L.muted }}>containers</div><div style={{ fontFamily: SERIF, fontSize: 40, color: L.muted }}>6</div><div style={{ fontFamily: SERIF, fontSize: 40 }}>1</div>
              <div style={{ color: L.muted }}>images</div><div style={{ fontFamily: SERIF, fontSize: 40, color: L.muted }}>1.4 GB</div><div style={{ fontFamily: SERIF, fontSize: 40 }}>44 MB</div>
              <div style={{ color: L.muted }}>memory, idle</div><div style={{ fontFamily: SERIF, fontSize: 40, color: L.muted }}>1.1 GB</div><div style={{ fontFamily: SERIF, fontSize: 40 }}>13 MB</div>
            </div>
          </Fade>
          <Caption dark={false} at={S(1.4)}>Measured idle on the same machine, same tool. Method in docs/BENCHMARKS.md.</Caption>
        </Sequence>
      </Shot>

      <Shot from={S(34)} dur={S(4)} zoom={false}>
        <EndCard dark={false} />
      </Shot>
    </AbsoluteFill>
  );
};


// ══════════════════════════════════════════════════════════════════════════════
// Film 3 · « Mix » — 34 s : terminal d'abord, schéma à la fin, sombre partout.
// ══════════════════════════════════════════════════════════════════════════════
export const SOBER_MIX_FRAMES = S(34);

export const SoberMix: React.FC = () => (
  <AbsoluteFill style={{ background: D.bg, color: D.text, fontFamily: SANS }}>
    <Shot from={0} dur={S(2.6)} zoom={false}>
      <TitleCard title="One binary for every notification." sub={<>docker compose up -d <span style={{ color: "#555" }}>· email, sms, push, in-app, telegram, slack, discord</span></>} />
    </Shot>

    {/* 1 · up, 5 s */}
    <Shot from={S(2.6)} dur={S(5)}>
      <Screen>
        <Terminal>
          <Line at={0} type="docker compose up -d" cps={70} />
          <Line at={S(0.8)}><M> Container notifyd-notifyd-db-1  Healthy</M></Line>
          <Line at={S(1)}><M> Container notifyd-notifyd-1     Started</M></Line>
          <Line at={S(1.4)} type="curl localhost:3400/v1/health" cps={70} />
          <Line at={S(2.2)}>{"{"}<K>"status"</K>: <V>"ok"</V>, <K>"db"</K>: <V>"ok"</V>, <K>"version"</K>: <V>"0.2.2"</V>{"}"}</Line>
        </Terminal>
      </Screen>
      <Caption at={S(2.6)}>Postgres is the only dependency. Up in the time it takes to read this.</Caption>
    </Shot>

    {/* 2 · send, 5.4 s */}
    <Shot from={S(7.6)} dur={S(5.4)}>
      <Screen>
        <Terminal>
          <Line at={0} type="curl -X POST localhost:3400/v1/send -d '{" cps={80} />
          <Line at={S(0.8)}>{"  "}<K>"subscriber_id"</K>: <V>"cust_4821"</V>,</Line>
          <Line at={S(0.95)}>{"  "}<K>"channels"</K>: [<V>"email"</V>, <V>"in_app"</V>, <V>"telegram"</V>],</Line>
          <Line at={S(1.1)}>{"  "}<K>"subject"</K>: <V>"Your order shipped"</V>,</Line>
          <Line at={S(1.25)}>{"  "}<K>"body"</K>: <V>"Hi {"{{first_name}}"}, parcel FR-2041 is on its way."</V> {"}'"}</Line>
          <Line at={S(2)}>{"{"}<K>"success"</K>: true, <K>"channels"</K>: [<V>"email"</V>, <V>"in_app"</V>, <V>"telegram"</V>],</Line>
          <Line at={S(2.15)}>{" "}<K>"job_ids"</K>: [<V>"cc67293b-…"</V>, <V>"a1579543-…"</V>, <V>"d98e7597-…"</V>]{"}"}</Line>
        </Terminal>
      </Screen>
      <Caption at={S(2.6)}>One call. The customer's email, inbox and Telegram, from the record you already keep.</Caption>
    </Shot>

    {/* 3 · campaign + 429 + digest, 7 s */}
    <Shot from={S(13)} dur={S(7)}>
      <Screen>
        <Terminal>
          <Line at={0} type={`curl -X POST localhost:3400/v1/batch -d '{"segment": {"has_email": true}, "template": "september-news"}'`} cps={110} />
          <Line at={S(1.4)}>{"{"}<K>"jobs_created"</K>: 41, <K>"subscribers"</K>: 41{"}"}</Line>
          <Line at={S(2)} type="notifyd digest" cps={70} />
          <Line at={S(2.6)}><M>Instance: email resend · </M><A>paused lanes: email</A></Line>
          <Line at={S(2.9)}><A>warning</A> — Lane email is paused after a provider 429.</Line>
          <Line at={S(3.1)}><M>  Transient. If it repeats, lower the lane's rate or ask the provider for a higher limit.</M></Line>
          <Line at={S(3.5)}><M>Queue</M>  pending 41 · retry 1 · processing 0</Line>
          <Line at={S(4.6)} type="notifyd jobs --status sent --since 1m" cps={70} />
          <Line at={S(5.4)}><V>44 job(s)</V><M> · email 42 via resend · in_app 1 · telegram 1</M></Line>
        </Terminal>
      </Screen>
      <Caption at={S(3.8)}>The provider says slow down. The lane waits exactly 47 seconds, urgent mail first. Nothing is dropped.</Caption>
    </Shot>

    {/* 4 · diagram, dark, 5.4 s */}
    <Shot from={S(20)} dur={S(5.4)} zoom={false}>
      <div style={{ position: "absolute", inset: 0, transform: "translateX(200px)" }}>
        <Node dark x={140} y={440} at={0} label="your app" sub="POST /v1/send" />
        <Wire x={400} y={484} w={160} at={4} color={D.line} />
        <Node dark x={560} y={400} w={360} h={170} at={6} label="notifyd" sub="one binary · Postgres" accent={D.text} />
        <Wire x={920} y={484} w={140} at={10} color={D.line} />
        <Fade at={12} style={{ position: "absolute", left: 1059, top: 202, height: 646, borderLeft: `2px solid ${D.line}` }} dy={0}><span /></Fade>
        {CHANNELS.map((c, i) => (
          <React.Fragment key={c}>
            <Wire x={1060} y={202 + i * 92} w={120} at={12 + i * 2} color={D.line} />
            <Node dark x={1180} y={170 + i * 92} w={240} h={64} at={14 + i * 3} label={c} mono accent={c === "email" ? D.green : undefined} sub={c === "email" ? "41 sent" : undefined} />
          </React.Fragment>
        ))}
        <Node dark x={560} y={140} w={360} h={88} at={S(1.6)} label="your agent" sub="MCP tools" accent={D.yellow} />
        <Fade at={S(1.8)} style={{ position: "absolute", left: 739, top: 228, height: 172, borderLeft: `2px dashed ${D.yellow}` }} dy={0}><span /></Fade>
        <Fade at={S(2.4)} style={{ position: "absolute", left: 140, top: 150, width: 380 }}>
          <div style={{ background: D.panel, border: `1.5px solid ${D.line}`, borderRadius: 14, padding: "16px 20px", fontFamily: SANS, fontSize: 21, lineHeight: 1.4, color: D.text }}>
            <div style={{ fontFamily: MONO, fontSize: 16, color: D.muted, marginBottom: 6 }}>Telegram · 19:07 · notifyd</div>
            notifyd: 1 warning. Lane email paused after a provider 429. Transient, nothing lost.
          </div>
        </Fade>
      </div>
      <Caption at={S(2.8)}>No dashboard. Your agent operates it over MCP, and the digest lands in your chat.</Caption>
    </Shot>

    {/* 5 · measured, 4.6 s */}
    <Shot from={S(25.4)} dur={S(4.6)} zoom={false}>
      <AbsoluteFill style={{ justifyContent: "center", alignItems: "center" }}>
        <div style={{ display: "grid", gridTemplateColumns: "300px 420px 420px", rowGap: 22, columnGap: 40, fontFamily: SANS, fontSize: 34, alignItems: "baseline" }}>
          <Fade at={0}><span /></Fade>
          <Fade at={0}><div style={{ fontFamily: MONO, fontSize: 24, color: D.muted, letterSpacing: 2 }}>NOVU 3.19</div></Fade>
          <Fade at={4}><div style={{ fontFamily: MONO, fontSize: 24, color: D.yellow, letterSpacing: 2 }}>NOTIFYD 0.2.2</div></Fade>
          {[
            ["containers", "6", "1"],
            ["images to pull", "1.4 GB", "44 MB"],
            ["memory, idle", "1.1 GB", "13 MB"],
          ].map(([k, a, b], i) => (
            <React.Fragment key={k}>
              <Fade at={8 + i * 6}><div style={{ color: D.muted }}>{k}</div></Fade>
              <Fade at={10 + i * 6}><div style={{ fontFamily: SERIF, fontSize: 56, color: D.muted }}>{a}</div></Fade>
              <Fade at={12 + i * 6}><div style={{ fontFamily: SERIF, fontSize: 56, color: D.text }}>{b}</div></Fade>
            </React.Fragment>
          ))}
        </div>
      </AbsoluteFill>
      <Caption at={S(1.4)}>Novu's own docker-compose, idle, measured on the same machine with the same tool. Method in docs/BENCHMARKS.md.</Caption>
    </Shot>

    <Shot from={S(30)} dur={S(4)} zoom={false}>
      <EndCard />
    </Shot>
  </AbsoluteFill>
);


// ══════════════════════════════════════════════════════════════════════════════
// Film 4 · « Final » — 31 s, chaque image est le schéma ou le terminal ; le
// titre est en surimpression du schéma ; gros caractères pour le fil X mobile.
// ══════════════════════════════════════════════════════════════════════════════
export const SOBER_FINAL_FRAMES = S(31);

const BigCaption: React.FC<{ children: React.ReactNode; at?: number }> = ({ children, at = 16 }) => (
  <Fade at={at} style={{ position: "absolute", left: 0, right: 0, bottom: 64, textAlign: "center", fontFamily: SERIF, fontSize: 48, lineHeight: 1.3, color: D.text, padding: "0 200px" }}>
    {children}
  </Fade>
);

/** Terminal du film final : 30 px, lignes courtes. */
const BigTerminal: React.FC<{ children: React.ReactNode }> = ({ children }) => (
  <div style={{ width: 1720, background: D.panel, border: `1px solid ${D.line}`, borderRadius: 14, boxShadow: "0 40px 120px rgba(0,0,0,.55)", overflow: "hidden" }}>
    <div style={{ display: "flex", alignItems: "center", gap: 8, padding: "12px 16px", borderBottom: `1px solid ${D.line}`, background: "#202020" }}>
      <span style={{ width: 12, height: 12, borderRadius: 6, background: "#3a3a3a" }} />
      <span style={{ width: 12, height: 12, borderRadius: 6, background: "#3a3a3a" }} />
      <span style={{ width: 12, height: 12, borderRadius: 6, background: "#3a3a3a" }} />
      <span style={{ marginLeft: "auto", marginRight: "auto", fontFamily: MONO, fontSize: 20, color: D.muted }}>Terminal — notifyd</span>
    </div>
    <pre style={{ margin: 0, padding: "28px 32px", fontFamily: MONO, fontSize: 34, lineHeight: 1.5, color: D.text, whiteSpace: "pre-wrap", textAlign: "left", minHeight: 560 }}>{children}</pre>
  </div>
);

const BigLine: React.FC<{ at: number; children?: React.ReactNode; type?: string; cps?: number }> = ({ at, children, type, cps = 90 }) => {
  const f = useCurrentFrame();
  if (f < at) return null;
  if (type !== undefined) {
    const n = Math.min(type.length, Math.floor(((f - at) / FPS) * cps));
    return (
      <div>
        <span style={{ color: D.muted }}>$ </span>
        {type.slice(0, n)}
        {n < type.length && <span style={{ display: "inline-block", width: 16, height: 34, background: D.text, verticalAlign: "-6px", marginLeft: 2 }} />}
      </div>
    );
  }
  const p = ease(interpolate(f, [at, at + 6], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" }));
  return <div style={{ opacity: p }}>{children}</div>;
};

/** Le schéma sombre, réutilisé en ouverture et en fin. `agent` ajoute la couche MCP. */
const DarkDiagram: React.FC<{ agent?: boolean; emailSub?: string; quiet?: boolean }> = ({ agent, emailSub, quiet }) => (
  <div style={{ position: "absolute", inset: 0, transform: "translateX(180px) translateY(40px)" }}>
    <Node dark mono={quiet} x={120} y={440} w={280} h={96} at={0} label="your app" sub="POST /v1/send" />
    <Wire x={400} y={488} w={160} at={4} color={D.line} />
    <Node dark mono={quiet} x={560} y={396} w={380} h={184} at={6} label="notifyd" sub="one binary · Postgres" accent={D.text} />
    <Wire x={940} y={488} w={120} at={10} color={D.line} />
    <Fade at={12} style={{ position: "absolute", left: 1059, top: 206, height: 646, borderLeft: `2px solid ${D.line}` }} dy={0}><span /></Fade>
    {CHANNELS.map((c, i) => (
      <React.Fragment key={c}>
        <Wire x={1060} y={206 + i * 92} w={120} at={12 + i * 2} color={D.line} />
        <Node dark x={1180} y={172 + i * 92} w={280} h={68} at={14 + i * 3} label={c} mono accent={c === "email" && emailSub ? (quiet ? D.text : D.green) : undefined} sub={c === "email" ? emailSub : undefined} />
      </React.Fragment>
    ))}
    {agent && (
      <>
        <Node dark mono={quiet} x={560} y={150} w={380} h={92} at={S(0.6)} label="your agent" sub="MCP tools" accent={D.yellow} />
        <Fade at={S(0.8)} style={{ position: "absolute", left: 749, top: 242, height: 154, borderLeft: `2px dashed ${D.yellow}` }} dy={0}><span /></Fade>
        <Fade at={S(1.3)} style={{ position: "absolute", left: 100, top: 130, width: 420 }}>
          <div style={{ background: D.panel, border: `1.5px solid ${D.line}`, borderRadius: 14, padding: "18px 22px", fontFamily: quiet ? SERIF : SANS, fontSize: quiet ? 29 : 27, lineHeight: 1.4, color: D.text }}>
            <div style={{ fontFamily: MONO, fontSize: 20, color: D.muted, marginBottom: 6 }}>Telegram · 19:07 · notifyd</div>
            1 warning. Lane email paused after a provider 429. Transient, nothing lost.
          </div>
        </Fade>
      </>
    )}
  </div>
);

/**
 * `quiet` : la variante « une seule couleur forte » (règles Apple relues le
 * 11 septembre) : terminal blanc et gris, jaune sur un seul élément par plan,
 * deux polices (serif pour la prose, mono pour tout ce qui est interface),
 * plan de mesure sur fond nu, fondus enchaînés à la place des passages au noir.
 */
const Final: React.FC<{ quiet?: boolean }> = ({ quiet = false }) => {
  const K: React.FC<{ children: React.ReactNode }> = ({ children }) => <span style={{ color: quiet ? D.text : D.blue }}>{children}</span>;
  const V: React.FC<{ children: React.ReactNode }> = ({ children }) => <span style={{ color: quiet ? D.text : D.green }}>{children}</span>;
  const Hi: React.FC<{ children: React.ReactNode }> = ({ children }) => <span style={{ color: quiet ? D.yellow : D.green }}>{children}</span>;
  const Warn: React.FC<{ children: React.ReactNode }> = ({ children }) => <span style={{ color: quiet ? D.yellow : D.amber }}>{children}</span>;
  const PROSE = quiet ? SERIF : SANS;
  const x = quiet;
  return (
  <AbsoluteFill style={{ background: D.bg, color: D.text, fontFamily: PROSE }}>
    {/* 0 · schéma + titre en surimpression, 3 s */}
    <Shot from={0} dur={S(3)} zoom={false}>
      <div style={{ position: "absolute", inset: 0, opacity: quiet ? 0.5 : 0.6, transform: "translate(520px, 140px) scale(0.72)", transformOrigin: "0 0" }}><DarkDiagram quiet={quiet} /></div>
      <Fade at={4} style={{ position: "absolute", left: 120, top: 110, display: "flex", flexDirection: "column", gap: 22 }}>
        <Mark size={52} />
        <div style={{ fontFamily: SERIF, fontSize: 96, lineHeight: 1.08, color: D.text, letterSpacing: -1, maxWidth: 1100 }}>One binary for every notification.</div>
        <div style={{ fontFamily: MONO, fontSize: 30, color: D.muted }}>self-hosted · Postgres only · operated by your agent</div>
      </Fade>
    </Shot>

    {/* 1 · up */}
    <Shot from={S(3)} dur={S(4.4)} xfade={x}>
      <Screen>
        <BigTerminal>
          <BigLine at={0} type="docker compose up -d" />
          <BigLine at={S(0.6)}><M> Container notifyd-db  Healthy</M></BigLine>
          <BigLine at={S(0.8)}><M> Container notifyd     Started</M></BigLine>
          <BigLine at={S(1.2)} type="curl localhost:3400/v1/health" />
          <BigLine at={S(1.9)}>{"{"}<K>"status"</K>: <Hi>"ok"</Hi>, <K>"db"</K>: <Hi>"ok"</Hi>, <K>"version"</K>: <V>"0.2.2"</V>{"}"}</BigLine>
        </BigTerminal>
      </Screen>
      <BigCaption at={S(2.2)}>Postgres is the only dependency.</BigCaption>
    </Shot>

    {/* 2 · send */}
    <Shot from={S(7.4)} dur={S(4.8)} xfade={x}>
      <Screen>
        <BigTerminal>
          <BigLine at={0} type="curl -X POST localhost:3400/v1/send -d '{" cps={110} />
          <BigLine at={S(0.6)}>{"  "}<K>"subscriber_id"</K>: <V>"cust_4821"</V>,</BigLine>
          <BigLine at={S(0.75)}>{"  "}<K>"channels"</K>: [<V>"email"</V>, <V>"in_app"</V>, <V>"telegram"</V>],</BigLine>
          <BigLine at={S(0.9)}>{"  "}<K>"subject"</K>: <V>"Your order shipped"</V>,</BigLine>
          <BigLine at={S(1.05)}>{"  "}<K>"body"</K>: <V>"Hi {"{{first_name}}"}, parcel FR-2041 is on its way."</V> {"}'"}</BigLine>
          <BigLine at={S(1.7)}>{"{"}<K>"success"</K>: true, <K>"channels"</K>: [<Hi>"email"</Hi>, <Hi>"in_app"</Hi>, <Hi>"telegram"</Hi>],</BigLine>
          <BigLine at={S(1.85)}>{" "}<K>"job_ids"</K>: [<V>"cc67293b…"</V>, <V>"a1579543…"</V>, <V>"d98e7597…"</V>]{"}"}</BigLine>
        </BigTerminal>
      </Screen>
      <BigCaption at={S(2.2)}>One call. Email, inbox and Telegram, from the customer you already know.</BigCaption>
    </Shot>

    {/* 3 · campaign, 429, digest, jobs */}
    <Shot from={S(12.2)} dur={S(6.6)} xfade={x}>
      <Screen>
        <BigTerminal>
          <BigLine at={0} type={`curl -X POST localhost:3400/v1/batch -d '{"segment": {"has_email": true}, "template": "news"}'`} cps={140} />
          <BigLine at={S(1)}>{"{"}<K>"jobs_created"</K>: 41, <K>"subscribers"</K>: 41{"}"}</BigLine>
          <BigLine at={S(1.5)} type="notifyd digest" />
          <BigLine at={S(2)}><Warn>warning</Warn>  Lane email is paused after a provider 429.</BigLine>
          <BigLine at={S(2.2)}><M>         Transient. Nothing lost, urgent mail goes first.</M></BigLine>
          <BigLine at={S(2.5)}><M>queue</M>    pending 41 · retry 1 · processing 0</BigLine>
          <BigLine at={S(3.6)} type="notifyd jobs --status sent --since 1m" />
          <BigLine at={S(4.3)}><V>44 job(s)</V><M>  email 42 via resend · in_app 1 · telegram 1</M></BigLine>
        </BigTerminal>
      </Screen>
      <BigCaption at={S(3)}>{quiet ? "The provider says slow down. The lane waits 47 seconds. Nothing lost." : "The provider says slow down. The lane waits exactly 47 seconds. Nothing is dropped."}</BigCaption>
    </Shot>

    {/* 4 · schéma : agent + Telegram */}
    <Shot from={S(18.8)} dur={S(4.6)} zoom={false} xfade={x}>
      <DarkDiagram agent emailSub="41 sent" quiet={quiet} />
      <BigCaption at={S(2)}>No dashboard. Your agent runs it over MCP. The digest lands in your chat.</BigCaption>
    </Shot>

    {/* 5 · schéma + mesure */}
    <Shot from={S(23.4)} dur={S(4.4)} zoom={false} xfade={x}>
      {!quiet && <div style={{ position: "absolute", inset: 0, opacity: 0.35 }}><DarkDiagram emailSub="41 sent" /></div>}
      <Fade at={0} style={{ position: "absolute", left: 0, right: 0, top: quiet ? 250 : 150, display: "flex", justifyContent: "center" }}>
        <div style={{ display: "grid", gridTemplateColumns: quiet ? "380px 420px 420px" : "340px 380px 380px", rowGap: quiet ? 26 : 18, columnGap: 40, alignItems: "baseline", background: quiet ? "transparent" : "rgba(20,20,20,.92)", border: quiet ? "none" : `1px solid ${D.line}`, borderRadius: 18, padding: quiet ? "0" : "34px 48px" }}>
          <span />
          <div style={{ fontFamily: MONO, fontSize: 26, color: D.muted, letterSpacing: 2 }}>NOVU 3.19</div>
          <div style={{ fontFamily: MONO, fontSize: 26, color: D.yellow, letterSpacing: 2 }}>NOTIFYD</div>
          {[["containers", "6", "1"], ["images to pull", "1.4 GB", "44 MB"], ["memory, idle", "1.1 GB", "13 MB"]].map(([k, a, b], i) => (
            <React.Fragment key={k}>
              <Fade at={6 + i * 6}><div style={{ fontFamily: PROSE, fontSize: quiet ? 38 : 34, color: D.muted }}>{k}</div></Fade>
              <Fade at={8 + i * 6}><div style={{ fontFamily: SERIF, fontSize: quiet ? 76 : 64, color: D.muted }}>{a}</div></Fade>
              <Fade at={10 + i * 6}><div style={{ fontFamily: SERIF, fontSize: quiet ? 76 : 64, color: D.text }}>{b}</div></Fade>
            </React.Fragment>
          ))}
        </div>
      </Fade>
      <BigCaption at={S(1.6)}>{quiet ? "Same machine, both idle. Method in docs/BENCHMARKS.md." : "Novu's own compose, idle, same machine, same tool. Method in docs/BENCHMARKS.md."}</BigCaption>
    </Shot>

    {/* 6 · terminal : la commande */}
    <Shot from={S(27.8)} dur={S(3.2)} zoom={false} xfade={x}>
      <Screen>
        <BigTerminal>
          <BigLine at={0} type="git clone https://github.com/rmzlb/notifyd && cd notifyd" cps={120} />
          <BigLine at={S(0.9)} type="docker compose up -d" cps={120} />
          <BigLine at={S(1.5)}><V>→ notifyd listening on :3400</V></BigLine>
        </BigTerminal>
      </Screen>
      <Fade at={S(1.2)} style={{ position: "absolute", left: 0, right: 0, bottom: 64, display: "flex", justifyContent: "center", alignItems: "center", gap: 22 }}>
        <Mark size={56} />
        <div style={{ fontFamily: SERIF, fontSize: 52, color: D.text }}>notifyd</div>
        <div style={{ fontFamily: MONO, fontSize: 28, color: D.muted }}>MIT · github.com/rmzlb/notifyd</div>
      </Fade>
    </Shot>
  </AbsoluteFill>
  );
};

export const SoberFinal: React.FC = () => <Final />;
export const SoberFinalQuiet: React.FC = () => <Final quiet />;
