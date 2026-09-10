import React from "react";
import { AbsoluteFill, Sequence, interpolate, useCurrentFrame, useVideoConfig } from "remotion";
import { C, font, mono } from "./theme";
import { Appear, Logo, Typewriter, useIn } from "./ui";

// Quatre formats courts (15 s, 1920×1080) pour X : une idée par plan, gros
// caractères, ambiance terminal, lisibles sans le son et sans être développeur.
// Toutes les données sont inventées sauf les mesures (docs/BENCHMARKS.md).

const FPS = 30;
const S = (sec: number) => Math.round(sec * FPS);
export const SHORT_FRAMES = S(15);

export const Stage: React.FC<{ children: React.ReactNode }> = ({ children }) => (
  <AbsoluteFill style={{ background: C.bg, color: C.text, fontFamily: font, overflow: "hidden" }}>
    <Grid />
    {children}
  </AbsoluteFill>
);

/** Fine grille de fond, façon terminal, qui donne de la profondeur sans distraire. */
export const Grid: React.FC = () => (
  <AbsoluteFill
    style={{
      backgroundImage: `linear-gradient(${C.line}22 1px, transparent 1px), linear-gradient(90deg, ${C.line}22 1px, transparent 1px)`,
      backgroundSize: "64px 64px",
      maskImage: "radial-gradient(ellipse at center, black 40%, transparent 85%)",
    }}
  />
);

export const Center: React.FC<{ children: React.ReactNode; gap?: number; pad?: number }> = ({ children, gap = 36, pad = 120 }) => (
  <AbsoluteFill style={{ padding: pad, display: "flex", flexDirection: "column", justifyContent: "center", alignItems: "center", gap, textAlign: "center" }}>{children}</AbsoluteFill>
);

export const Big: React.FC<{ children: React.ReactNode; size?: number; color?: string; weight?: number }> = ({ children, size = 84, color = C.text, weight = 800 }) => (
  <div style={{ fontSize: size, fontWeight: weight, letterSpacing: -2.5, lineHeight: 1.05, color }}>{children}</div>
);

export const Caption: React.FC<{ children: React.ReactNode; color?: string }> = ({ children, color = C.muted }) => (
  <div style={{ fontSize: 40, color, lineHeight: 1.3, maxWidth: 1500 }}>{children}</div>
);

export const Term: React.FC<{ children: React.ReactNode; width?: number; size?: number; title?: string }> = ({ children, width = 1500, size = 34, title = "shell" }) => (
  <div style={{ width, background: C.panel, border: `2px solid ${C.line}`, borderRadius: 18, boxShadow: "0 30px 80px rgba(0,0,0,.5)", overflow: "hidden" }}>
    <div style={{ display: "flex", alignItems: "center", gap: 10, padding: "14px 20px", borderBottom: `1px solid ${C.line}`, background: "#12151c" }}>
      <span style={{ width: 14, height: 14, borderRadius: 7, background: "#ff5f57" }} />
      <span style={{ width: 14, height: 14, borderRadius: 7, background: "#febc2e" }} />
      <span style={{ width: 14, height: 14, borderRadius: 7, background: "#28c840" }} />
      <span style={{ marginLeft: 14, fontFamily: mono, fontSize: 20, color: C.muted }}>{title}</span>
    </div>
    <pre style={{ margin: 0, padding: "26px 32px", fontFamily: mono, fontSize: size, lineHeight: 1.5, whiteSpace: "pre-wrap", color: C.text, textAlign: "left" }}>{children}</pre>
  </div>
);

export const Y: React.FC<{ children: React.ReactNode }> = ({ children }) => <span style={{ color: C.yellow }}>{children}</span>;
export const G: React.FC<{ children: React.ReactNode }> = ({ children }) => <span style={{ color: C.green }}>{children}</span>;
export const R: React.FC<{ children: React.ReactNode }> = ({ children }) => <span style={{ color: C.red }}>{children}</span>;
export const M: React.FC<{ children: React.ReactNode }> = ({ children }) => <span style={{ color: C.muted }}>{children}</span>;
export const B: React.FC<{ children: React.ReactNode }> = ({ children }) => <span style={{ color: C.blue }}>{children}</span>;

/** Compteur qui monte avec une courbe douce. */
export const Count: React.FC<{ from: number; to: number; start: number; end: number; suffix?: string; color?: string; size?: number }> = ({ from, to, start, end, suffix = "", color = C.text, size = 120 }) => {
  const frame = useCurrentFrame();
  const v = Math.round(interpolate(frame, [start, end], [from, to], { extrapolateLeft: "clamp", extrapolateRight: "clamp", easing: (t) => 1 - Math.pow(1 - t, 3) }));
  return <div style={{ fontFamily: mono, fontSize: size, fontWeight: 800, color, letterSpacing: -3, fontVariantNumeric: "tabular-nums" }}>{v.toLocaleString("en").replace(/,/g, " ")}{suffix}</div>;
};

export const Outro: React.FC<{ line: string }> = ({ line }) => (
  <Center gap={28}>
    <Appear><div style={{ display: "flex", alignItems: "center", gap: 30 }}><Logo size={120} /><div style={{ fontSize: 132, fontWeight: 800, letterSpacing: -6 }}>notify<Y>d</Y></div></div></Appear>
    <Appear delay={8}><Caption color={C.text}>{line}</Caption></Appear>
    <Appear delay={16}><Caption>Open source · MIT · <span style={{ color: C.yellow, fontFamily: mono }}>github.com/rmzlb/notifyd</span></Caption></Appear>
  </Center>
);

/** Fondu de sortie d'une scène, pour enchaîner sans coupe sèche. */
export const Scene: React.FC<{ from: number; dur: number; children: React.ReactNode }> = ({ from, dur, children }) => {
  const Inner: React.FC = () => {
    const frame = useCurrentFrame();
    const o = interpolate(frame, [0, 8, dur - 10, dur], [0, 1, 1, 0], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
    return <AbsoluteFill style={{ opacity: o }}>{children}</AbsoluteFill>;
  };
  return <Sequence from={from} durationInFrames={dur}><Inner /></Sequence>;
};

// ── 1 · Un appel, quatre canaux ──────────────────────────────────────────────
/** Icônes vectorielles (le navigateur de rendu n'a pas de police emoji). */
export const Icon: React.FC<{ kind: "mail" | "sms" | "push" | "inbox"; size?: number; color?: string }> = ({ kind, size = 72, color = C.yellow }) => {
  const common = { width: size, height: size, viewBox: "0 0 24 24", fill: "none", stroke: color, strokeWidth: 1.6, strokeLinecap: "round" as const, strokeLinejoin: "round" as const };
  if (kind === "mail") return <svg {...common}><rect x="3" y="5" width="18" height="14" rx="2.5" /><path d="M3.5 7.5 12 13l8.5-5.5" /></svg>;
  if (kind === "sms") return <svg {...common}><path d="M20 12.5a7.5 7.5 0 0 1-7.5 7.5H5l-1.5 1.5V12.5A7.5 7.5 0 0 1 11 5h1.5a7.5 7.5 0 0 1 7.5 7.5Z" /><path d="M8.5 12.5h7" /></svg>;
  if (kind === "push") return <svg {...common}><rect x="7" y="2.5" width="10" height="19" rx="2.5" /><path d="M11 18.5h2" /><path d="M9.5 6.5h5" /></svg>;
  return <svg {...common}><path d="M6 17V11a6 6 0 1 1 12 0v6l1.5 2H4.5L6 17Z" /><path d="M10 21a2 2 0 0 0 4 0" /></svg>;
};

export const Channel: React.FC<{ icon: "mail" | "sms" | "push" | "inbox"; name: string; detail: string; delay: number }> = ({ icon, name, detail, delay }) => {
  const p = useIn(delay);
  const done = useIn(delay + 18);
  return (
    <div style={{ opacity: p, transform: `translateY(${(1 - p) * 40}px) scale(${0.9 + p * 0.1})`, width: 330, height: 300, background: C.panel, border: `2px solid ${C.line}`, borderRadius: 24, padding: 28, display: "flex", flexDirection: "column", justifyContent: "space-between", boxShadow: "0 20px 60px rgba(0,0,0,.45)" }}>
      <div><Icon kind={icon} /></div>
      <div>
        <div style={{ fontSize: 40, fontWeight: 800 }}>{name}</div>
        <div style={{ fontSize: 24, color: C.muted, fontFamily: mono, marginTop: 6 }}>{detail}</div>
      </div>
      <div style={{ fontSize: 30, color: C.green, fontWeight: 700, opacity: done, transform: `scale(${0.6 + done * 0.4})` }}>✓ delivered</div>
    </div>
  );
};

export const ShortOneCall: React.FC = () => (
  <Stage>
    <Scene from={0} dur={S(3)}>
      <Center>
        <Appear><Big>Every product has to<br />send notifications.</Big></Appear>
        <Appear delay={14}><Caption>Email, texts, push, in-app. Usually four services and a dashboard.</Caption></Appear>
      </Center>
    </Scene>
    <Scene from={S(3)} dur={S(5.5)}>
      <Center gap={30}>
        <Appear><Caption color={C.text}>With notifyd, it is <b>one call</b>.</Caption></Appear>
        <Appear delay={6}>
          <Term title="your-app">
            <M>$ </M><Typewriter start={10} cps={70} text={`curl notifyd/v1/send -d '{`} />{"\n"}
            {"  "}<B>"to"</B>: <G>"alice"</G>,{"\n"}
            {"  "}<B>"channels"</B>: [<G>"email"</G>, <G>"sms"</G>, <G>"push"</G>, <G>"in_app"</G>],{"\n"}
            {"  "}<B>"body"</B>: <G>"Your order has shipped"</G>{"\n"}
            {"}'"}{"\n"}
            <Sequence from={S(3.4)} layout="none"><span><G>→ 202 accepted</G> <M>· 4 jobs · 3 ms</M></span></Sequence>
          </Term>
        </Appear>
      </Center>
    </Scene>
    <Scene from={S(8.5)} dur={S(4)}>
      <Center gap={44}>
        <div style={{ display: "flex", gap: 30 }}>
          <Channel icon="mail" name="Email" detail="resend · 0.6 s" delay={0} />
          <Channel icon="sms" name="SMS" detail="telnyx · 1.1 s" delay={8} />
          <Channel icon="push" name="Push" detail="fcm · 0.3 s" delay={16} />
          <Channel icon="inbox" name="In-app" detail="live · 12 ms" delay={24} />
        </div>
        <Appear delay={40}><Caption color={C.text}>Your own providers. Retries, priorities and failover built in.</Caption></Appear>
      </Center>
    </Scene>
    <Scene from={S(12.5)} dur={S(2.5)}>
      <Outro line="One Rust binary. Postgres only. Every channel." />
    </Scene>
  </Stage>
);

// ── 2 · Rien ne se perd ──────────────────────────────────────────────────────
export const Provider: React.FC<{ name: string; state: "ok" | "down" | "idle" | "active"; delay?: number }> = ({ name, state, delay = 0 }) => {
  const p = useIn(delay);
  const color = state === "down" ? C.red : state === "active" ? C.yellow : state === "ok" ? C.green : C.muted;
  const label = state === "down" ? "429 · refusing" : state === "active" ? "delivering" : state === "ok" ? "delivering" : "standby";
  return (
    <div style={{ opacity: p, width: 520, background: C.panel, border: `3px solid ${color}`, borderRadius: 22, padding: "26px 34px", display: "flex", justifyContent: "space-between", alignItems: "center", boxShadow: state === "down" ? `0 0 60px ${C.red}55` : "none" }}>
      <div style={{ fontSize: 44, fontWeight: 800 }}>{name}</div>
      <div style={{ fontFamily: mono, fontSize: 28, color }}>{label}</div>
    </div>
  );
};

export const ShortNothingLost: React.FC = () => {
  const frame = useCurrentFrame();
  const phaseDown = frame >= S(3.2) && frame < S(6.4);
  const total = 4812;
  const sent = frame < S(3.2)
    ? Math.round(interpolate(frame, [S(0.6), S(3.2)], [0, 1900], { extrapolateLeft: "clamp", extrapolateRight: "clamp" }))
    : frame < S(6.4) ? 1900 : Math.round(interpolate(frame, [S(6.4), S(10.2)], [1900, total], { extrapolateLeft: "clamp", extrapolateRight: "clamp" }));
  return (
    <Stage>
      <Scene from={0} dur={S(10.6)}>
        <AbsoluteFill style={{ padding: "90px 120px", display: "flex", flexDirection: "column", gap: 34 }}>
          <Appear>
            <Caption color={C.text}>Friday, 6 pm. Sending a campaign to <b>4 812</b> customers.</Caption>
          </Appear>
          <div style={{ display: "flex", alignItems: "center", gap: 40 }}>
            <div style={{ fontFamily: mono, fontSize: 150, fontWeight: 800, letterSpacing: -6, fontVariantNumeric: "tabular-nums", color: phaseDown ? C.muted : C.text }}>{sent.toLocaleString("en").replace(/,/g, " ")}</div>
            <div style={{ fontSize: 40, color: C.muted }}>/ 4 812 sent</div>
          </div>
          <div style={{ height: 22, background: C.line, borderRadius: 11, overflow: "hidden" }}>
            <div style={{ width: `${(sent / total) * 100}%`, height: "100%", background: phaseDown ? C.red : C.green, transition: "none" }} />
          </div>
          <div style={{ display: "flex", gap: 30, marginTop: 10 }}>
            <Provider name="Resend" state={frame < S(3.2) ? "ok" : "down"} />
            <Sequence from={S(6)} layout="none"><Provider name="SMTP fallback" state="active" /></Sequence>
          </div>
          <Sequence from={S(3.4)} layout="none">
            <Appear><Caption color={C.red}>Your email provider starts refusing. <span style={{ color: C.muted }}>Most tools: retries pile up, some mails are lost.</span></Caption></Appear>
          </Sequence>
          <Sequence from={S(6.6)} layout="none">
            <Appear><Caption color={C.yellow}>notifyd pauses the lane, keeps the count, and switches provider. Password resets go first.</Caption></Appear>
          </Sequence>
        </AbsoluteFill>
      </Scene>
      <Scene from={S(10.6)} dur={S(2.4)}>
        <Center gap={40}>
          <Appear><div style={{ display: "flex", gap: 90 }}>
            <div><Big size={140} color={C.green}>0</Big><Caption>lost</Caption></div>
            <div><Big size={140}>13 <span style={{ fontSize: 60 }}>MB</span></Big><Caption>RAM</Caption></div>
            <div><Big size={140}>44k</Big><Caption>jobs / second</Caption></div>
          </div></Appear>
          <Appear delay={12}><Caption>Measured. Method and hardware in docs/BENCHMARKS.md.</Caption></Appear>
        </Center>
      </Scene>
      <Scene from={S(13)} dur={S(2)}>
        <Outro line="Notifications that do not lose mail when a provider does." />
      </Scene>
    </Stage>
  );
};

// ── 3 · Ton agent est d'astreinte ────────────────────────────────────────────
export const Bubble: React.FC<{ who: "you" | "agent"; children: React.ReactNode; delay: number }> = ({ who, children, delay }) => {
  const p = useIn(delay);
  const you = who === "you";
  return (
    <div style={{ opacity: p, transform: `translateY(${(1 - p) * 30}px)`, alignSelf: you ? "flex-end" : "flex-start", maxWidth: 1250, background: you ? C.yellow : C.panel, color: you ? "#111" : C.text, border: `2px solid ${you ? C.yellow : C.line}`, borderRadius: 26, padding: "26px 36px", fontSize: 40, lineHeight: 1.35 }}>
      {children}
    </div>
  );
};

export const ToolCall: React.FC<{ delay: number; call: string; result: string; ok?: boolean }> = ({ delay, call, result, ok = true }) => {
  const p = useIn(delay);
  const r = useIn(delay + 16);
  return (
    <div style={{ opacity: p, alignSelf: "flex-start", fontFamily: mono, fontSize: 30, color: C.muted, paddingLeft: 24, borderLeft: `4px solid ${C.line}`, lineHeight: 1.5 }}>
      <div><span style={{ color: C.blue }}>▸ notifyd.</span>{call}</div>
      <div style={{ opacity: r, color: ok ? C.green : C.red }}>{result}</div>
    </div>
  );
};

export const ShortAgentOnCall: React.FC = () => (
  <Stage>
    <Scene from={0} dur={S(12.6)}>
      <AbsoluteFill style={{ padding: "80px 140px", display: "flex", flexDirection: "column", gap: 26 }}>
        <Appear><div style={{ fontFamily: mono, fontSize: 26, color: C.muted }}>claude code · 09:14 · notifyd MCP connected</div></Appear>
        <Bubble who="you" delay={6}>Alice says she never got her invoice email. What happened?</Bubble>
        <ToolCall delay={S(1.6)} call={`list_jobs(recipient: "alice@…", since: "24h")`} result={`1 job · failed at 09:12 · 550 mailbox full`} ok={false} />
        <ToolCall delay={S(3.6)} call={`retry_job("job_8f31")`} result={`delivered ✓ 0.8 s`} />
        <Bubble who="agent" delay={S(5.6)}>Her mailbox was full at 09:12. I resent it at 09:14 and it was delivered. No other failures today.</Bubble>
        <Sequence from={S(8.2)} layout="none">
          <Appear><Caption color={C.text}>No dashboard to learn. The person on call is your agent, and every action is audited.</Caption></Appear>
        </Sequence>
      </AbsoluteFill>
    </Scene>
    <Scene from={S(12.6)} dur={S(2.4)}>
      <Outro line="Send through it. Let your agent run it." />
    </Scene>
  </Stage>
);

// ── 4 · Moins ───────────────────────────────────────────────────────────────
export const Block: React.FC<{ label: string; delay: number; color?: string; small?: boolean }> = ({ label, delay, color = C.line, small }) => {
  const p = useIn(delay);
  return (
    <div style={{ opacity: p, transform: `translateY(${(1 - p) * -40}px)`, width: small ? 320 : 460, padding: small ? "18px 26px" : "22px 30px", background: C.panel, border: `2px solid ${color}`, borderRadius: 16, fontFamily: mono, fontSize: small ? 30 : 34, textAlign: "center" }}>
      {label}
    </div>
  );
};

export const ShortLess: React.FC = () => (
  <Stage>
    <Scene from={0} dur={S(12.6)}>
      <AbsoluteFill style={{ padding: "80px 140px", display: "flex", flexDirection: "row", gap: 120, alignItems: "center" }}>
        <div style={{ flex: 1, display: "flex", flexDirection: "column", alignItems: "center", gap: 14 }}>
          <Appear><Caption>The usual self-hosted notification stack</Caption></Appear>
          {["react dashboard", "api", "worker", "websocket", "Redis", "MongoDB"].map((l, i) => <Block key={l} label={l} delay={10 + i * 8} />)}
          <Sequence from={S(2.6)} layout="none"><Appear><Caption color={C.red}>six services to babysit</Caption></Appear></Sequence>
        </div>
        <div style={{ flex: 1, display: "flex", flexDirection: "column", alignItems: "center", gap: 18 }}>
          <Sequence from={S(3.4)} layout="none">
            <Appear><Caption color={C.text}>notifyd</Caption></Appear>
            <Block label="one 10 MB binary" delay={6} color={C.yellow} small />
            <Block label="PostgreSQL" delay={14} small />
            <Appear delay={26}><Caption color={C.green}>13 MB of RAM at idle</Caption></Appear>
          </Sequence>
          <Sequence from={S(6.4)} layout="none">
            <Appear>
              <div style={{ display: "flex", flexWrap: "wrap", gap: 12, justifyContent: "center", maxWidth: 760, marginTop: 20 }}>
                {["email", "sms", "whatsapp", "push", "in-app inbox", "workflows", "preferences", "retries", "failover", "send windows", "MCP"].map((f, i) => (
                  <Appear key={f} delay={i * 4}><span style={{ display: "inline-block", border: `2px solid ${C.line}`, borderRadius: 999, padding: "10px 22px", fontSize: 30, color: C.text, background: C.panel }}>{f}</span></Appear>
                ))}
              </div>
            </Appear>
          </Sequence>
          <Sequence from={S(9.6)} layout="none"><Appear><Caption color={C.text}>Same job. Nothing else to run.</Caption></Appear></Sequence>
        </div>
      </AbsoluteFill>
    </Scene>
    <Scene from={S(12.6)} dur={S(2.4)}>
      <Outro line="Less to run, nothing missing." />
    </Scene>
  </Stage>
);
