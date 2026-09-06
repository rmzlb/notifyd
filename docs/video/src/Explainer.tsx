import React from "react";
import { AbsoluteFill, Sequence, interpolate, useCurrentFrame } from "remotion";
import { C, mono } from "./theme";
import { Appear, B, Chip, Code, Frame, G, Logo, M, R, Title, Typewriter, Y, useIn } from "./ui";

const FPS = 30;
const S = (sec: number) => Math.round(sec * FPS);

// Scene lengths (seconds)
const L = { intro: 5, problem: 8, send: 10, lanes: 10, agent: 14, numbers: 7, outro: 6 };
const starts = (() => {
  let t = 0; const o: Record<keyof typeof L, number> = {} as never;
  (Object.keys(L) as (keyof typeof L)[]).forEach((k) => { o[k] = S(t); t += L[k]; });
  return { ...o, total: S(t) };
})();
export const TOTAL_FRAMES = starts.total;

const Pad: React.FC<{ children: React.ReactNode; top?: boolean; gap?: number }> = ({ children, top, gap = 28 }) => (
  <div style={{ position: "absolute", inset: 0, padding: top ? "44px 88px" : "64px 88px", display: "flex", flexDirection: "column", justifyContent: top ? "flex-start" : "center", gap }}>{children}</div>
);

const Intro: React.FC = () => (
  <Pad>
    <Appear><div style={{ display: "flex", alignItems: "center", gap: 28 }}><Logo size={96} /><div style={{ fontSize: 92, fontWeight: 800, letterSpacing: -4 }}>notify<Y>d</Y></div></div></Appear>
    <Appear delay={12}><div style={{ fontSize: 38, color: C.text, lineHeight: 1.3 }}>The notification service your agent can <b>send through</b> and <b>run</b>.</div></Appear>
    <Appear delay={24}><div style={{ fontSize: 28, color: C.muted }}>Email · SMS · WhatsApp · push · in-app inbox — one Rust binary, Postgres only.</div></Appear>
  </Pad>
);

const Box: React.FC<{ title: string; lines: string[]; accent: string; delay: number; dim?: boolean }> = ({ title, lines, accent, delay, dim }) => (
  <Appear delay={delay} style={{ flex: 1 }}>
    <div style={{ border: `2px solid ${dim ? C.line : accent}`, borderRadius: 16, padding: "22px 26px", background: C.panel, height: 300, opacity: dim ? 0.75 : 1 }}>
      <div style={{ fontSize: 26, fontWeight: 700, color: accent, marginBottom: 14 }}>{title}</div>
      {lines.map((l) => <div key={l} style={{ fontSize: 23, color: C.text, lineHeight: 1.5 }}>{l}</div>)}
    </div>
  </Appear>
);

const Problem: React.FC = () => (
  <Pad>
    <Appear><Title small>Every product sends notifications. The usual options:</Title></Appear>
    <div style={{ display: "flex", gap: 24 }}>
      <Box delay={10} title="Hosted SaaS" accent={C.muted} dim lines={["billed per notification", "your data on their side", "dashboard-first"]} />
      <Box delay={20} title="Heavy self-hosted" accent={C.muted} dim lines={["MongoDB + Redis", "4 containers", "React dashboard to babysit"]} />
      <Box delay={30} title="notifyd" accent={C.yellow} lines={["one 10 MB binary", "PostgreSQL only", "no dashboard: digest + MCP"]} />
    </div>
  </Pad>
);

const Send: React.FC = () => {
  const frame = useCurrentFrame();
  const p = interpolate(frame, [S(4.2), S(5.2)], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
  return (
    <Pad>
      <Appear><Title small>1 · Send in one call</Title></Appear>
      <div style={{ display: "flex", gap: 28, alignItems: "stretch" }}>
        <div style={{ flex: 1.15 }}>
          <Appear delay={6}>
            <Code size={20}>
              <M>$ </M>curl -X POST https://notifyd.shop-eu.example/v1/send \{"\n"}
              {"    "}-H <G>"X-Api-Key: sk_shop_eu_…"</G> -d '{"{"}{"\n"}
              {"      "}<B>"channels"</B>: [<G>"email"</G>, <G>"in_app"</G>],{"\n"}
              {"      "}<B>"subscriber_id"</B>: <G>"cust-48213"</G>,{"\n"}
              {"      "}<B>"subject"</B>: <G>"Your order #EU-77410 has shipped"</G>,{"\n"}
              {"      "}<B>"body"</B>: <G>"Hi {"{{first_name}}"}, it left our warehouse today."</G>,{"\n"}
              {"      "}<B>"priority"</B>: <G>"normal"</G>,{"\n"}
              {"      "}<B>"idempotency_key"</B>: <G>"order-EU-77410-shipped"</G>{"\n"}
              {"    "}{"}"}'
            </Code>
          </Appear>
        </div>
        <div style={{ flex: 0.85, display: "flex", flexDirection: "column", gap: 14, justifyContent: "center" }}>
          <Appear delay={S(2)}><Chip>flat REST · <Y>curl is the SDK</Y></Chip></Appear>
          <Appear delay={S(2.6)}><Chip>retries safe: <Y>idempotency_key</Y></Chip></Appear>
          <Appear delay={S(3.2)}><Chip>schedule: <Y>scheduled_at</Y></Chip></Appear>
          <Appear delay={S(3.8)}><Chip>campaign: <Y>POST /v1/batch</Y></Chip></Appear>
          <div style={{ opacity: p, transform: `translateY(${(1 - p) * 10}px)`, fontFamily: mono, fontSize: 20, color: C.green, marginTop: 8 }}>
            → 202 {"{"} "job_ids": ["1cf6…eede"], "success": true {"}"}
          </div>
        </div>
      </div>
    </Pad>
  );
};

type Lane = { name: "critical" | "normal" | "bulk"; color: string; prio: string };
const Lanes: React.FC = () => {
  const frame = useCurrentFrame();
  const lanes: Lane[] = [
    { name: "critical", color: C.red, prio: "10" }, { name: "normal", color: C.blue, prio: "50" }, { name: "bulk", color: C.purple, prio: "80" },
  ];
  // provider 429 on the email channel at 5.2 s, channel paused until 8.0 s
  const paused = frame >= S(5.2) && frame < S(8.0);
  const before = interpolate(frame, [S(1.5), S(5.2)], [0, 2150], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
  const after = frame >= S(8.6) ? interpolate(frame, [S(8.6), S(10)], [0, 900], { extrapolateRight: "clamp" }) : 0;
  const bulkCount = Math.min(5000, Math.floor(before + after));
  const resetQueued = frame >= S(6.4) && frame < S(8.0);
  const resetSent = frame >= S(8.0) && frame < S(9.6);
  const stateFor = (l: Lane): { text: string; color: string; width: string } => {
    if (l.name === "bulk") return paused
      ? { text: "waiting: email channel paused", color: C.muted, width: `${(bulkCount / 5000) * 100}%` }
      : { text: `${bulkCount.toLocaleString("en")} / 5,000 of campaign "autumn-serums" sent`, color: C.muted, width: `${(bulkCount / 5000) * 100}%` };
    if (l.name === "critical") return resetQueued
      ? { text: "password reset for cust-9917 queued — head of the line", color: C.yellow, width: "12%" }
      : resetSent ? { text: "channel resumed → password reset delivered first, 0.4 s", color: C.green, width: "100%" } : { text: "idle", color: C.muted, width: "0%" };
    return { text: paused ? "waiting: email channel paused" : "order updates, 3 in flight", color: C.muted, width: "38%" };
  };
  return (
    <Pad>
      <Appear><Title small>2 · A real delivery engine, not a loop over an array</Title></Appear>
      <div style={{ border: `2px solid ${paused ? C.red : C.line}`, borderRadius: 16, padding: "12px 14px", background: paused ? "#1a1114" : "transparent", display: "flex", flexDirection: "column", gap: 12 }}>
        <div style={{ display: "flex", justifyContent: "space-between", fontFamily: mono, fontSize: 19, color: paused ? C.red : C.muted, padding: "0 6px" }}>
          <span>channel <b style={{ color: C.text }}>email</b> · provider resend · 8 msg/s token bucket</span>
          <span>{paused ? "PAUSED · 429 Too Many Requests · Retry-After: 30s · attempt not consumed" : "flowing"}</span>
        </div>
        {lanes.map((l, i) => {
          const st = stateFor(l);
          return (
            <Appear key={l.name} delay={8 + i * 6}>
              <div style={{ display: "flex", alignItems: "center", gap: 18, border: `2px solid ${C.line}`, borderRadius: 14, padding: "12px 20px", background: C.panel }}>
                <div style={{ width: 150, fontFamily: mono, fontSize: 22, color: l.color, fontWeight: 700 }}>{l.name} <M>·{l.prio}</M></div>
                <div style={{ flex: 1, height: 14, background: "#0b0d11", borderRadius: 7, overflow: "hidden" }}>
                  <div style={{ width: st.width, height: "100%", background: paused ? C.line : l.color }} />
                </div>
                <div style={{ width: 480, fontSize: 19, color: st.color }}>{st.text}</div>
              </div>
            </Appear>
          );
        })}
      </div>
      <div style={{ display: "flex", gap: 14, flexWrap: "wrap" }}>
        <Appear delay={S(3)}><Chip>claim order: <Y>priority ASC, scheduled_at ASC</Y></Chip></Appear>
        <Appear delay={S(5.4)}><Chip color={C.red}>429 pauses the channel for Retry-After — a fallback provider is tried first</Chip></Appear>
        <Appear delay={S(8.2)}><Chip color={C.green}>on resume, critical leaves before the campaign</Chip></Appear>
        <Appear delay={S(9)}><Chip>backoff 30s → 2h with jitter · stuck-job reaper</Chip></Appear>
      </div>
    </Pad>
  );
};

const Bubble: React.FC<{ who: "you" | "agent" | "tool"; children: React.ReactNode; delay: number }> = ({ who, children, delay }) => {
  const p = useIn(delay);
  const right = who === "you";
  const bg = who === "you" ? C.yellow : who === "tool" ? "#0b0d11" : C.panel;
  const color = who === "you" ? "#0f1115" : C.text;
  return (
    <div style={{ display: "flex", justifyContent: right ? "flex-end" : "flex-start", opacity: p, transform: `translateY(${(1 - p) * 16}px)` }}>
      <div style={{ maxWidth: 980, background: bg, color, border: `2px solid ${who === "tool" ? C.line : "transparent"}`, borderRadius: 16, padding: "10px 16px", fontSize: who === "tool" ? 17 : 21, fontFamily: who === "tool" ? mono : undefined, lineHeight: 1.45, whiteSpace: "pre-wrap" }}>{children}</div>
    </div>
  );
};

const Agent: React.FC = () => (
  <Pad top gap={16}>
    <Appear><Title small>3 · No dashboard. Your agent runs it, over MCP.</Title></Appear>
    <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
      <Bubble who="you" delay={S(0.6)}>Anything wrong on the notification side this morning?</Bubble>
      <Bubble who="tool" delay={S(1.8)}><M>▶ notifyd · </M><Y>digest</Y><M>(window: "24h")</M>{"\n"}
        <R>warning</R>  Bounce rate 5.3 % over the window (14 bounced / 263 delivered) — project <B>shop-eu</B>, template <B>autumn-serums</B>{"\n"}
        <R>warning</R>  Primary email provider `resend` is resting for 47s after refusing messages; `smtp` is delivering.{"\n"}
        <M>info   </M>  Lane `bulk` paused 3× on provider 429 (longest 41 s). Pacing is doing its job.</Bubble>
      <Bubble who="agent" delay={S(5.2)}>Two things need you. The bounces come from one imported list; I can suppress the 14 addresses (marketing scope only, transactional keeps flowing). The provider blip resolved itself through the SMTP fallback, nothing was lost.</Bubble>
      <Bubble who="you" delay={S(8.2)}>Do it, and move the campaign window to 9–18h Paris.</Bubble>
      <Bubble who="tool" delay={S(9.6)}><M>▶ </M><Y>add_suppression</Y><M> × 14 (scope: marketing)</M>   <G>✓</G>{"\n"}
        <M>▶ </M><Y>update_project</Y><M>(shop-eu, send_window: 09:00–18:00 Europe/Paris)</M>   <G>✓</G>{"\n"}
        <M>▶ </M><Y>send_test</Y><M>(shop-eu, email → ops@shop-eu.example)</M>   <G>sent · resend · 0.6 s</G></Bubble>
      <Appear delay={S(11.6)}><div style={{ fontSize: 19, color: C.muted }}>13 tools · <Y>readOnlyHint</Y> / <Y>destructiveHint</Y> annotations · read-only key for agents that report but must not act · every call audited</div></Appear>
    </div>
  </Pad>
);

const Num: React.FC<{ v: string; l: string; delay: number }> = ({ v, l, delay }) => (
  <Appear delay={delay} style={{ flex: 1 }}>
    <div style={{ border: `2px solid ${C.line}`, borderRadius: 16, padding: "26px 24px", background: C.panel, textAlign: "center" }}>
      <div style={{ fontSize: 60, fontWeight: 800, color: C.yellow, letterSpacing: -2 }}>{v}</div>
      <div style={{ fontSize: 22, color: C.text, marginTop: 6 }}>{l}</div>
    </div>
  </Appear>
);

const Numbers: React.FC = () => (
  <Pad>
    <Appear><Title small>Footprint, measured (8 Arm vCPU, untuned Postgres 16)</Title></Appear>
    <div style={{ display: "flex", gap: 20 }}>
      <Num v="42 MB" l="container image" delay={8} />
      <Num v="13 MB" l="RSS at idle" delay={14} />
      <Num v="23 MB" l="RSS draining 100k jobs" delay={20} />
    </div>
    <div style={{ display: "flex", gap: 20 }}>
      <Num v="44k/s" l="jobs enqueued (batch)" delay={26} />
      <Num v="3.5k/s" l="jobs drained (batching provider)" delay={32} />
      <Num v="70 ms" l="digest over 100k jobs" delay={38} />
    </div>
    <Appear delay={48}><div style={{ fontSize: 21, color: C.muted }}>Only numbers we measured on notifyd itself. Method and scripts: docs/BENCHMARKS.md</div></Appear>
  </Pad>
);

const Outro: React.FC = () => (
  <Pad>
    <Appear><div style={{ display: "flex", alignItems: "center", gap: 24 }}><Logo size={80} /><div style={{ fontSize: 72, fontWeight: 800, letterSpacing: -3 }}>notify<Y>d</Y></div></div></Appear>
    <Appear delay={10}><Code size={26} style={{ display: "inline-block" }}>docker pull ghcr.io/rmzlb/notifyd{"\n"}cargo install notifyd{"\n"}nix run github:rmzlb/notifyd</Code></Appear>
    <Appear delay={22}><div style={{ fontSize: 30 }}>github.com/rmzlb/notifyd <M>· MIT · MCP registry: io.github.rmzlb/notifyd</M></div></Appear>
  </Pad>
);

const Scene: React.FC<{ from: number; len: number; children: React.ReactNode }> = ({ from, len, children }) => {
  return (
    <Sequence from={from} durationInFrames={len}>
      <SceneFade len={len}>{children}</SceneFade>
    </Sequence>
  );
};
const SceneFade: React.FC<{ len: number; children: React.ReactNode }> = ({ len, children }) => {
  const f = useCurrentFrame();
  const o = interpolate(f, [0, 8, len - 10, len], [0, 1, 1, 0], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
  return <AbsoluteFill style={{ opacity: o }}>{children}</AbsoluteFill>;
};

const Progress: React.FC = () => {
  const f = useCurrentFrame();
  return <div style={{ position: "absolute", left: 0, bottom: 0, height: 5, width: `${(f / TOTAL_FRAMES) * 100}%`, background: C.yellow }} />;
};

export const Explainer: React.FC = () => (
  <Frame>
    <Scene from={starts.intro} len={S(L.intro)}><Intro /></Scene>
    <Scene from={starts.problem} len={S(L.problem)}><Problem /></Scene>
    <Scene from={starts.send} len={S(L.send)}><Send /></Scene>
    <Scene from={starts.lanes} len={S(L.lanes)}><Lanes /></Scene>
    <Scene from={starts.agent} len={S(L.agent)}><Agent /></Scene>
    <Scene from={starts.numbers} len={S(L.numbers)}><Numbers /></Scene>
    <Scene from={starts.outro} len={S(L.outro)}><Outro /></Scene>
    <Progress />
  </Frame>
);
