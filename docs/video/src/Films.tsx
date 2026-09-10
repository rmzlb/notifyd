import React from "react";
import { AbsoluteFill, Img, Sequence, interpolate, staticFile, useCurrentFrame } from "remotion";
import { C, font, mono } from "./theme";
import { Appear, Typewriter, useIn } from "./ui";
import { B, Big, Bubble, Caption, Center, Channel, G, M, Scene, Stage, Term, ToolCall, Y } from "./Shorts";

// Trois films pour X (1920×1080, 30 fps), sans limite de durée : un mélange
// des angles « un appel », « ton agent est d'astreinte » et « moins ». Gros
// caractères, une idée par plan, lisibles sans le son et sans être développeur.
// Données inventées sauf les mesures (docs/BENCHMARKS.md).

const FPS = 30;
const S = (sec: number) => Math.round(sec * FPS);

/** Marque : `REMOTION_LOGO=badge|prompt|n-dot|mail-badge` choisit le fichier de public/logos. */
const LOGO = process.env.REMOTION_LOGO ?? "nd-two";

export const Mark: React.FC<{ size?: number; tile?: boolean }> = ({ size = 96, tile = false }) => (
  <Img src={staticFile(`logos/${LOGO}${tile ? "" : "-bare"}.svg`)} style={{ width: size, height: size, display: "block" }} />
);

/** Lockup « notify● » quand la marque est le badge (le d est dedans), « ● notifyd » sinon. */
export const Brand: React.FC<{ size?: number }> = ({ size = 132 }) => (
  <div style={{ display: "flex", alignItems: "center", gap: size * 0.12 }}>
    {LOGO === "badge" ? (
      <>
        <div style={{ fontSize: size, fontWeight: 800, letterSpacing: -size * 0.045, lineHeight: 1 }}>notify</div>
        <Mark size={size * 0.86} />
      </>
    ) : (
      <>
        <Mark size={size * 0.95} tile={!LOGO.startsWith("nd-")} />
        <div style={{ fontSize: size, fontWeight: 800, letterSpacing: -size * 0.045, lineHeight: 1 }}>
          notify<Y>d</Y>
        </div>
      </>
    )}
  </div>
);

const End: React.FC<{ line: string }> = ({ line }) => (
  <Center gap={30}>
    <Appear><Brand /></Appear>
    <Appear delay={8}><Caption color={C.text}>{line}</Caption></Appear>
    <Appear delay={16}><Caption>Open source · MIT · <span style={{ color: C.yellow, fontFamily: mono }}>github.com/rmzlb/notifyd</span></Caption></Appear>
  </Center>
);

/** Repère de chapitre en haut à gauche. */
const Kicker: React.FC<{ n: string; label: string }> = ({ n, label }) => (
  <Appear style={{ position: "absolute", top: 60, left: 90, display: "flex", alignItems: "center", gap: 18, fontFamily: mono, fontSize: 28, color: C.muted }}>
    <span style={{ width: 18, height: 18, background: C.yellow, borderRadius: 4, display: "inline-block" }} />
    <span><span style={{ color: C.text }}>{n}</span> · {label}</span>
  </Appear>
);

/** Balayage jaune entre deux chapitres. */
const Wipe: React.FC<{ at: number }> = ({ at }) => {
  const frame = useCurrentFrame();
  const x = interpolate(frame, [at, at + 16], [-110, 110], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
  if (frame < at || frame > at + 16) return null;
  return <AbsoluteFill style={{ transform: `translateX(${x}%) skewX(-12deg)`, background: C.yellow, zIndex: 10 }} />;
};

/** Arrivée « claquée » : gros, puis à sa taille. */
const Slam: React.FC<{ delay?: number; children: React.ReactNode; style?: React.CSSProperties }> = ({ delay = 0, children, style }) => {
  const p = useIn(delay, 18);
  return <div style={{ opacity: Math.min(1, p * 1.4), transform: `scale(${1.5 - p * 0.5})`, ...style }}>{children}</div>;
};

/** Carte de service (le « avant »). */
const Tile: React.FC<{ label: string; sub: string; delay: number; x: number; y: number; gone: number }> = ({ label, sub, delay, x, y, gone }) => {
  const frame = useCurrentFrame();
  const p = useIn(delay, 14);
  const g = interpolate(frame, [gone, gone + 14], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
  return (
    <div style={{ position: "absolute", left: 960 + x * (1 - g) - 200, top: 540 + y * (1 - g) - 90, width: 400, height: 180, opacity: p * (1 - g), transform: `translateY(${(1 - p) * -120}px) scale(${1 - g * 0.6})`, background: C.panel, border: `2px solid ${C.line}`, borderRadius: 22, padding: "26px 30px", boxShadow: "0 24px 60px rgba(0,0,0,.45)" }}>
      <div style={{ fontSize: 40, fontWeight: 800 }}>{label}</div>
      <div style={{ fontFamily: mono, fontSize: 24, color: C.muted, marginTop: 8 }}>{sub}</div>
    </div>
  );
};

/** Numéro qui monte, format « 12 400 ». */
const num = (v: number) => Math.round(v).toLocaleString("en").replace(/,/g, " ");

// ══════════════════════════════════════════════════════════════════════════════
// Film A · « One » — trois chapitres : Aujourd'hui · Un appel · 3 h 07
// ══════════════════════════════════════════════════════════════════════════════
export const FILM_ONE_FRAMES = S(42);

const TILES = [
  ["Email service", "container · queue · retries"],
  ["SMS service", "container · rate limits"],
  ["Push service", "container · device tokens"],
  ["Queue", "Redis"],
  ["Database", "MongoDB"],
  ["Dashboard", "React app · logins"],
];
const TILE_POS = [[-460, -220], [0, -220], [460, -220], [-460, 60], [0, 60], [460, 60]];

export const FilmOne: React.FC = () => {
  return (
    <Stage>
      {/* Ouverture */}
      <Scene from={0} dur={S(3.4)}>
        <Center gap={40}>
          <Slam><Big size={110}>Your app has to tell people things.</Big></Slam>
          <div style={{ display: "flex", gap: 26 }}>
            {["a reset link", "a shipped parcel", "a red badge", "a 2FA code"].map((t, i) => (
              <Appear key={t} delay={16 + i * 7}><span style={{ display: "inline-block", border: `2px solid ${C.line}`, borderRadius: 999, padding: "14px 30px", fontSize: 36, background: C.panel }}>{t}</span></Appear>
            ))}
          </div>
        </Center>
      </Scene>

      {/* Chapitre 1 · Aujourd'hui */}
      <Scene from={S(3.4)} dur={S(8.6)}>
        <Kicker n="01" label="today" />
        {TILES.map(([l, s], i) => <Tile key={l} label={l} sub={s} delay={8 + i * 7} x={TILE_POS[i][0]} y={TILE_POS[i][1]} gone={S(4.6)} />)}
        <Sequence from={S(2.4)} durationInFrames={S(2.4)} layout="none">
          <Appear style={{ position: "absolute", left: 0, right: 0, top: 800, textAlign: "center" }}>
            <Caption color={C.text}>Six things to run. <span style={{ color: C.red }}>Six things that break at 3 a.m.</span></Caption>
          </Appear>
        </Sequence>
        <Sequence from={S(5)} layout="none">
          <Center gap={34}>
            <Slam>
              <div style={{ display: "flex", alignItems: "center", gap: 34 }}>
                <div style={{ background: C.yellow, color: "#111", borderRadius: 26, padding: "34px 50px", fontSize: 60, fontWeight: 800, boxShadow: `0 0 90px ${C.yellow}55` }}>notifyd <span style={{ fontFamily: mono, fontSize: 40, fontWeight: 600 }}>10 MB</span></div>
                <div style={{ fontSize: 60, color: C.muted }}>+</div>
                <div style={{ background: C.panel, border: `2px solid ${C.line}`, borderRadius: 26, padding: "34px 50px", fontSize: 60, fontWeight: 800 }}>Postgres</div>
              </div>
            </Slam>
            <Appear delay={14}><Caption color={C.text}>One binary. One database you already have. <span style={{ color: C.green }}>Nothing else.</span></Caption></Appear>
          </Center>
        </Sequence>
      </Scene>
      <Wipe at={S(12)} />

      {/* Chapitre 2 · Un appel */}
      <Scene from={S(12.2)} dur={S(10.4)}>
        <Kicker n="02" label="one call" />
        <Sequence from={0} durationInFrames={S(5.4)} layout="none">
          <Center gap={30}>
            <Appear><Caption color={C.text}>Your code makes <b>one call</b>.</Caption></Appear>
            <Appear delay={6}>
              <Term title="your-app" width={1560} size={36}>
                <M>$ </M><Typewriter start={8} cps={80} text={`curl notifyd/v1/send -d '{`} />{"\n"}
                {"  "}<B>"to"</B>: <G>"alice"</G>,{"\n"}
                {"  "}<B>"channels"</B>: [<G>"email"</G>, <G>"sms"</G>, <G>"push"</G>, <G>"in_app"</G>],{"\n"}
                {"  "}<B>"body"</B>: <G>"Your order has shipped"</G>{"\n"}
                {"}'"}{"\n"}
                <Sequence from={S(3.2)} layout="none"><span><G>→ 202 accepted</G> <M>· 4 jobs · 3 ms</M></span></Sequence>
              </Term>
            </Appear>
          </Center>
        </Sequence>
        <Sequence from={S(5.4)} layout="none">
          <Center gap={44}>
            <div style={{ display: "flex", gap: 30 }}>
              <Channel icon="mail" name="Email" detail="your provider · 0.6 s" delay={0} />
              <Channel icon="sms" name="SMS" detail="your provider · 1.1 s" delay={8} />
              <Channel icon="push" name="Push" detail="fcm · 0.3 s" delay={16} />
              <Channel icon="inbox" name="In-app" detail="live · 12 ms" delay={24} />
            </div>
            <Appear delay={40}><Caption color={C.text}>Every channel. Retries, priorities, quiet hours and provider failover <span style={{ color: C.yellow }}>included</span>.</Caption></Appear>
          </Center>
        </Sequence>
      </Scene>
      <Wipe at={S(22.4)} />

      {/* Chapitre 3 · 3 h 07 */}
      <Scene from={S(22.6)} dur={S(11)}>
        <Kicker n="03" label="3:07 a.m." />
        <AbsoluteFill style={{ padding: "150px 160px 80px", display: "flex", flexDirection: "column", gap: 26 }}>
          <Appear><div style={{ fontFamily: mono, fontSize: 26, color: C.muted }}>your agent · 03:07 · notifyd MCP connected · you are asleep</div></Appear>
          <ToolCall delay={S(0.8)} call={`digest()`} result={`warning · provider "resend" refused messages (429) · email lane paused 47 s · "smtp" is delivering · 0 lost`} ok={false} />
          <ToolCall delay={S(3.2)} call={`list_jobs(status: "failed", since: "1h")`} result={`0 jobs`} />
          <Bubble who="agent" delay={S(4.8)}>Resend asked us to slow down at 03:05. notifyd paused that lane for exactly 47 s and sent the urgent mail through SMTP. Nothing was lost, nothing to do.</Bubble>
          <Sequence from={S(7.4)} layout="none">
            <Appear><Caption color={C.text}>No dashboard. <span style={{ color: C.yellow }}>Built to be run by your agent, natively</span>: every operation is an MCP tool.</Caption></Appear>
          </Sequence>
        </AbsoluteFill>
      </Scene>

      {/* Chiffres */}
      <Scene from={S(33.6)} dur={S(4.2)}>
        <Center gap={40}>
          <div style={{ display: "flex", gap: 110 }}>
            <Slam><div><Big size={150}>13 <span style={{ fontSize: 60 }}>MB</span></Big><Caption>RAM at idle</Caption></div></Slam>
            <Slam delay={6}><div><Big size={150}>44 500</Big><Caption>sends / second accepted</Caption></div></Slam>
            <Slam delay={12}><div><Big size={150} color={C.green}>0</Big><Caption>lost when a provider fails</Caption></div></Slam>
          </div>
          <Appear delay={22}><Caption>Measured. Method and hardware in docs/BENCHMARKS.md.</Caption></Appear>
        </Center>
      </Scene>

      <Scene from={S(37.8)} dur={S(4.2)}>
        <End line="One binary. Every channel. Run by your agent." />
      </Scene>
    </Stage>
  );
};

// ══════════════════════════════════════════════════════════════════════════════
// Film B · « The conversation » — tout le film est un fil de discussion ;
// un bandeau de compteurs en haut réagit à ce qui se dit.
// ══════════════════════════════════════════════════════════════════════════════
export const FILM_TALK_FRAMES = S(46);

const Strip: React.FC = () => {
  const frame = useCurrentFrame();
  const t0 = S(6), tPause = S(17.5), tResume = S(21.5), tEnd = S(36);
  const queued = frame < S(4.5) ? 0 : 12400;
  const sent = frame < t0 ? 0
    : frame < tPause ? interpolate(frame, [t0, tPause], [0, 5400], { extrapolateRight: "clamp" })
    : frame < tResume ? 5400
    : interpolate(frame, [tResume, tEnd], [5400, 12400], { extrapolateRight: "clamp", extrapolateLeft: "clamp" });
  const paused = frame >= tPause && frame < tResume;
  const left = Math.max(1, Math.ceil((47 * (tResume - frame)) / (tResume - tPause)));
  const state = frame < t0 ? "idle" : paused ? `email paused · ${left} s` : sent >= 12400 ? "done" : "sending";
  const color = paused ? C.yellow : state === "done" ? C.green : C.text;
  return (
    <div style={{ position: "absolute", top: 0, left: 0, right: 0, height: 96, background: "#0b0d11", borderBottom: `2px solid ${C.line}`, display: "flex", alignItems: "center", gap: 56, padding: "0 90px", fontFamily: mono, fontSize: 28, whiteSpace: "nowrap" }}>
      <span style={{ display: "flex", alignItems: "center", gap: 14 }}><Mark size={40} />notifyd</span>
      <span><span style={{ color: C.muted }}>queued</span> {num(queued)}</span>
      <span><span style={{ color: C.muted }}>sent</span> <span style={{ fontVariantNumeric: "tabular-nums" }}>{num(sent)}</span></span>
      <span><span style={{ color: C.muted }}>critical lane</span> <span style={{ color: C.green }}>flowing</span></span>
      <span style={{ marginLeft: "auto", color, fontWeight: 700 }}>{state}</span>
    </div>
  );
};

const Time: React.FC<{ t: string; delay: number }> = ({ t, delay }) => (
  <Appear delay={delay} style={{ alignSelf: "center", fontFamily: mono, fontSize: 24, color: C.muted }}>{t}</Appear>
);

export const FilmTalk: React.FC = () => (
  <Stage>
    <Sequence from={0} durationInFrames={S(41.2)} layout="none"><Strip /></Sequence>
    {/* Un seul fil : on fait défiler par blocs (trois écrans). */}
    <Scene from={0} dur={S(15.6)}>
      <AbsoluteFill style={{ padding: "150px 170px 80px", display: "flex", flexDirection: "column", gap: 24 }}>
        <Time t="09:41" delay={0} />
        <Bubble who="you" delay={6}>Send the launch email to our 12 400 customers today. Password resets must never wait behind it.</Bubble>
        <ToolCall delay={S(2)} call={`batch(template: "launch", subscribers: 12 400, priority: "bulk", send_window: "daytime")`} result={`12 400 queued · bulk lane · each in the customer's own daytime`} />
        <Bubble who="agent" delay={S(4.6)}>Queued. Each customer gets it during their daytime. Resets and codes are on the critical lane, so they always go first.</Bubble>
        <Sequence from={S(8)} layout="none">
          <Appear><Caption color={C.text}>One call for 12 400 people. <span style={{ color: C.yellow }}>Priorities and quiet hours are the server's job</span>, not yours.</Caption></Appear>
        </Sequence>
      </AbsoluteFill>
    </Scene>

    <Scene from={S(15.6)} dur={S(14)}>
      <AbsoluteFill style={{ padding: "150px 170px 80px", display: "flex", flexDirection: "column", gap: 24 }}>
        <Time t="11:06" delay={0} />
        <ToolCall delay={8} call={`digest()`} result={`warning · "resend" answered 429 · email lane paused 47 s, resumed in priority order · 0 failed`} ok={false} />
        <Bubble who="agent" delay={S(2.4)}>Heads-up: Resend asked us to slow down for 47 seconds. notifyd paused the lane, kept the count and resumed in priority order. Nothing lost, nothing to do.</Bubble>
        <Bubble who="you" delay={S(6)}>Did anything fail?</Bubble>
        <ToolCall delay={S(7.2)} call={`list_jobs(status: "failed", since: "today")`} result={`0 jobs`} />
        <Bubble who="agent" delay={S(8.8)}>No. 0 failed, 0 bounced so far.</Bubble>
        <Sequence from={S(10.6)} layout="none">
          <Appear><Caption color={C.text}>The agent did not read a dashboard. <span style={{ color: C.yellow }}>It called the same tools an operator would</span>, over MCP.</Caption></Appear>
        </Sequence>
      </AbsoluteFill>
    </Scene>

    <Scene from={S(29.6)} dur={S(11.6)}>
      <AbsoluteFill style={{ padding: "150px 170px 80px", display: "flex", flexDirection: "column", gap: 24 }}>
        <Time t="17:20" delay={0} />
        <Bubble who="you" delay={6}>What do we actually run for all this?</Bubble>
        <ToolCall delay={S(1.8)} call={`health()`} result={`ok · one binary · postgres · rss 13 MB · uptime 41 d`} />
        <Bubble who="agent" delay={S(3.6)}>One 10 MB binary and the Postgres you already had. 13 MB of RAM right now. No Redis, no dashboard, no third container.</Bubble>
        <Bubble who="you" delay={S(6.4)}>Perfect.</Bubble>
        <Sequence from={S(7.6)} layout="none">
          <Appear><Caption color={C.text}>Email, SMS, WhatsApp, push, in-app. <span style={{ color: C.green }}>Less to run, nothing missing.</span></Caption></Appear>
        </Sequence>
      </AbsoluteFill>
    </Scene>

    <Scene from={S(41.2)} dur={S(4.8)}>
      <End line="Send through it. Let your agent run it." />
    </Scene>
  </Stage>
);

// ══════════════════════════════════════════════════════════════════════════════
// Film C · « Numbers » — avant / après en écran partagé, cadence rapide.
// ══════════════════════════════════════════════════════════════════════════════
export const FILM_NUMBERS_FRAMES = S(33);

const ROWS: Array<[string, string]> = [
  ["6 services", "1 binary"],
  ["MongoDB + Redis", "just Postgres"],
  ["30+ min setup", "docker compose up"],
  ["a dashboard to host", "an API and MCP tools"],
  ["someone on call", "your agent on call"],
  ["a 429 fails the job", "paused 47 s, 0 lost"],
];

const Row: React.FC<{ left: string; right: string; delay: number }> = ({ left, right, delay }) => {
  const l = useIn(delay, 16);
  const r = useIn(delay + 7, 16);
  return (
    <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 60, alignItems: "center" }}>
      <div style={{ opacity: l, transform: `translateX(${(1 - l) * -80}px)`, textAlign: "right", fontSize: 64, fontWeight: 700, color: C.muted, textDecoration: r > 0.6 ? "line-through" : "none", textDecorationColor: C.red, textDecorationThickness: 5 }}>{left}</div>
      <div style={{ opacity: r, transform: `translateX(${(1 - r) * 80}px) scale(${0.85 + r * 0.15})`, fontSize: 64, fontWeight: 800, color: C.text }}>{right}</div>
    </div>
  );
};

export const FilmNumbers: React.FC = () => (
  <Stage>
    <Scene from={0} dur={S(2.6)}>
      <Center gap={30}>
        <Slam><Big size={120}>Notifications, two ways.</Big></Slam>
        <Appear delay={12}><Caption>Email · SMS · WhatsApp · push · in-app</Caption></Appear>
      </Center>
    </Scene>

    <Scene from={S(2.6)} dur={S(13)}>
      <AbsoluteFill style={{ padding: "110px 140px", display: "flex", flexDirection: "column", justifyContent: "center", gap: 40 }}>
        <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 60, fontFamily: mono, fontSize: 28, color: C.muted, letterSpacing: 2 }}>
          <Appear style={{ textAlign: "right" }}>THE USUAL STACK</Appear>
          <Appear delay={4} style={{ color: C.yellow }}>NOTIFYD</Appear>
        </div>
        {ROWS.map(([l, r], i) => <Row key={l} left={l} right={r} delay={12 + i * S(1.6)} />)}
      </AbsoluteFill>
    </Scene>
    <Wipe at={S(15.4)} />

    <Scene from={S(15.6)} dur={S(3.2)}>
      <Center gap={20}><Slam><Big size={200}>44 500</Big></Slam><Appear delay={8}><Caption color={C.text}>notifications per second <b>accepted</b>, on a laptop</Caption></Appear></Center>
    </Scene>
    <Scene from={S(18.8)} dur={S(3)}>
      <Center gap={20}><Slam><Big size={200}>13 <span style={{ fontSize: 80 }}>MB</span></Big></Slam><Appear delay={8}><Caption color={C.text}>of RAM at idle. <span style={{ color: C.muted }}>Yes, megabytes.</span></Caption></Appear></Center>
    </Scene>
    <Scene from={S(21.8)} dur={S(3)}>
      <Center gap={20}><Slam><Big size={200} color={C.green}>0</Big></Slam><Appear delay={8}><Caption color={C.text}>lost when your email provider says <span style={{ color: C.red }}>429</span>. Lane paused, count kept, failover tried.</Caption></Appear></Center>
    </Scene>

    <Scene from={S(24.8)} dur={S(4.6)}>
      <AbsoluteFill style={{ padding: "140px 170px 80px", display: "flex", flexDirection: "column", gap: 26 }}>
        <Appear><div style={{ fontFamily: mono, fontSize: 26, color: C.muted }}>your agent · notifyd MCP connected</div></Appear>
        <Bubble who="you" delay={6}>Anything wrong with notifications today?</Bubble>
        <ToolCall delay={S(1.2)} call={`digest()`} result={`0 findings · 12 400 sent · p95 2.1 s · bounce 0.2 %`} />
        <Bubble who="agent" delay={S(2.6)}>Nothing. 12 400 sent, no failures, no bounces to worry about.</Bubble>
      </AbsoluteFill>
    </Scene>

    <Scene from={S(29.4)} dur={S(3.6)}>
      <End line="Less to run. Nothing missing. Run by your agent." />
    </Scene>
  </Stage>
);

// ══════════════════════════════════════════════════════════════════════════════
// Film D · « Three ways » — pour qui, contre quoi (hébergé, Novu), ce qui change,
// en chiffres qui veulent dire quelque chose. Le flow rapide du film C.
// ══════════════════════════════════════════════════════════════════════════════
export const FILM_THREE_WAYS_FRAMES = S(48);

const Mark2: React.FC<{ ok: boolean }> = ({ ok }) => (
  <span style={{ display: "inline-block", width: 44, color: ok ? C.green : C.red, fontWeight: 800 }}>{ok ? "✓" : "✗"}</span>
);

const Line: React.FC<{ ok: boolean; text: string; delay: number }> = ({ ok, text, delay }) => {
  const p = useIn(delay, 16);
  return <div style={{ opacity: p, transform: `translateX(${(1 - p) * 60}px)`, fontSize: 54, fontWeight: 700, color: ok ? C.text : C.muted }}><Mark2 ok={ok} />{text}</div>;
};

const Stack: React.FC<{ label: string; delay: number }> = ({ label, delay }) => {
  const p = useIn(delay, 14);
  return <div style={{ opacity: p, transform: `translateY(${(1 - p) * -60}px)`, background: C.panel, border: `2px solid ${C.line}`, borderRadius: 16, padding: "16px 28px", fontFamily: mono, fontSize: 34, textAlign: "center", minWidth: 300 }}>{label}</div>;
};

const NOVU = ["api", "worker", "websocket", "web dashboard", "MongoDB", "Redis", "S3 storage"];

const CHANGES: Array<[string, string, string]> = [
  ["MongoDB + Redis + S3", "just the Postgres you already run", "nothing new to operate"],
  ["7 containers", "1 binary · 10 MB", "one thing to update"],
  ["a dashboard to log into", "an API, run by your agent over MCP", "no admin UI to host or learn"],
  ["billed per notification", "€0 per notification", "MIT · you pay your providers, nothing else"],
  ["a 429 fails the send", "paused · nothing lost · urgent first", "the engine handles the bad day"],
  ["a platform to babysit", "13 MB of RAM, next to your app", "fits the small server you already pay for"],
];

const Change: React.FC<{ left: string; right: string; why: string; delay: number }> = ({ left, right, why, delay }) => {
  const l = useIn(delay, 16);
  const r = useIn(delay + 6, 16);
  return (
    <div style={{ display: "grid", gridTemplateColumns: "1fr 1.35fr", gap: 50, alignItems: "center" }}>
      <div style={{ opacity: l, transform: `translateX(${(1 - l) * -60}px)`, textAlign: "right", fontSize: 42, fontWeight: 700, color: C.muted, textDecoration: r > 0.6 ? "line-through" : "none", textDecorationColor: C.red, textDecorationThickness: 4 }}>{left}</div>
      <div style={{ opacity: r, transform: `translateX(${(1 - r) * 60}px)` }}>
        <div style={{ fontSize: 44, fontWeight: 800, color: C.text }}>{right}</div>
        <div style={{ fontSize: 26, color: C.yellow, marginTop: 2 }}>{why}</div>
      </div>
    </div>
  );
};

export const FilmThreeWays: React.FC = () => (
  <Stage>
    {/* Pour qui */}
    <Scene from={0} dur={S(5.4)}>
      <Center gap={36}>
        <Slam><Big size={116}>You ship a product.</Big></Slam>
        <div style={{ display: "flex", gap: 24 }}>
          {["reset links", "shipped parcels", "2FA codes", "a red badge"].map((t, i) => (
            <Appear key={t} delay={12 + i * 6}><span style={{ display: "inline-block", border: `2px solid ${C.line}`, borderRadius: 999, padding: "12px 28px", fontSize: 34, background: C.panel }}>{t}</span></Appear>
          ))}
        </div>
        <Appear delay={44}><Caption color={C.text}>It has to reach people. <span style={{ color: C.yellow }}>You do not want to run a notification platform.</span></Caption></Appear>
      </Center>
    </Scene>

    {/* Option 1 · hébergé */}
    <Scene from={S(5.4)} dur={S(6.6)}>
      <Kicker n="option 1" label="hosted · Knock, Courier, SuprSend…" />
      <AbsoluteFill style={{ padding: "170px 200px 80px", display: "flex", flexDirection: "column", gap: 26, justifyContent: "center" }}>
        <Line ok text="nothing to run" delay={6} />
        <Line ok={false} text="billed per notification, forever" delay={20} />
        <Line ok={false} text="your customers' data leaves your servers" delay={34} />
        <Line ok={false} text="their dashboard, their rules, their outages" delay={48} />
        <Appear delay={70}><Caption>Fine at 1 000 users. The invoice grows faster than you do.</Caption></Appear>
      </AbsoluteFill>
    </Scene>
    <Wipe at={S(11.9)} />

    {/* Option 2 · Novu */}
    <Scene from={S(12.1)} dur={S(7.4)}>
      <Kicker n="option 2" label="self-hosted · Novu" />
      <Center gap={40}>
        <div style={{ display: "flex", flexWrap: "wrap", gap: 18, justifyContent: "center", maxWidth: 1500 }}>
          {NOVU.map((l, i) => <Stack key={l} label={l} delay={6 + i * 7} />)}
        </div>
        <Appear delay={70}><Caption color={C.text}><span style={{ color: C.red }}>Seven things</span> to run, patch and babysit. A dashboard to log into. <span style={{ color: C.muted }}>(their docker-compose, today)</span></Caption></Appear>
        <Appear delay={92}><Caption>Yours, but now you are running a platform.</Caption></Appear>
      </Center>
    </Scene>
    <Wipe at={S(19.4)} />

    {/* Option 3 · notifyd, ce qui change */}
    <Scene from={S(19.6)} dur={S(14.4)}>
      <Kicker n="option 3" label="notifyd · what changes" />
      <AbsoluteFill style={{ padding: "150px 150px 80px", display: "flex", flexDirection: "column", justifyContent: "center", gap: 30 }}>
        {CHANGES.map(([l, r, w], i) => <Change key={l} left={l} right={r} why={w} delay={6 + i * S(1.75)} />)}
      </AbsoluteFill>
    </Scene>
    <Wipe at={S(33.9)} />

    {/* L'agent */}
    <Scene from={S(34.1)} dur={S(6.6)}>
      <AbsoluteFill style={{ padding: "150px 170px 80px", display: "flex", flexDirection: "column", gap: 26 }}>
        <Appear><div style={{ fontFamily: mono, fontSize: 26, color: C.muted }}>your agent · notifyd MCP connected</div></Appear>
        <Bubble who="you" delay={6}>Anything wrong with notifications today?</Bubble>
        <ToolCall delay={S(1.2)} call={`digest()`} result={`0 findings · 12 400 sent · 0 failed · bounce 0.2 %`} />
        <Bubble who="agent" delay={S(2.6)}>Nothing. 12 400 sent, no failures. The provider slowed us for 47 s at 11:06; nothing was lost.</Bubble>
        <Sequence from={S(4.4)} layout="none"><Appear><Caption color={C.text}>No dashboard. <span style={{ color: C.yellow }}>Built to be run by your agent, natively.</span></Caption></Appear></Sequence>
      </AbsoluteFill>
    </Scene>

    {/* Pour qui, encore, et la marque */}
    <Scene from={S(40.7)} dur={S(7.3)}>
      <Center gap={26}>
        <Slam><Big size={72}>For the team that already runs Postgres,</Big></Slam>
        <Slam delay={10}><Big size={72}>already works with an agent,</Big></Slam>
        <Slam delay={20}><Big size={72} color={C.yellow}>and would rather ship than babysit.</Big></Slam>
        <Appear delay={44}><Brand size={110} /></Appear>
        <Appear delay={54}><Caption>In production for three companies · MIT · <span style={{ color: C.yellow, fontFamily: mono }}>github.com/rmzlb/notifyd</span></Caption></Appear>
      </Center>
    </Scene>
  </Stage>
);
