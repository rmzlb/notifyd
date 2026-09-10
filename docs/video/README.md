# Explainer video

60-second explainer rendered with [Remotion](https://www.remotion.dev): the
problem, one send call, priority lanes under a provider 429, an agent
operating the instance over MCP, measured footprint. All data shown is
invented (`shop-eu`, `cust-48213`, `autumn-serums`); the digest lines reuse
the real wording produced by `GET /v1/admin/digest`.

```bash
cd docs/video && npm install
npx remotion browser ensure          # headless Chrome for rendering
npm run render                        # out/notifyd-explainer.mp4 (1280×720, 30 fps)
npm run gif                           # docs/assets/notifyd-explainer.gif (880 px, 10 fps)
```

The MP4 is attached to the release it was made for (`gh release upload vX.Y.Z out/notifyd-explainer.mp4`) and the README links to that asset; the GIF is embedded in the README. Keep both under 10 MB.

## Shorts for X (15 s, 1920×1080)

Four compositions in `src/Shorts.tsx`, one idea each, terminal ambience but
readable by non-technical viewers. Same invented data policy as the explainer.

| Composition | Idea | Output |
|---|---|---|
| `ShortOneCall` | One call, four channels delivered | `out/short-one-call.mp4` |
| `ShortNothingLost` | A campaign hits a provider 429; nothing is lost | `out/short-nothing-lost.mp4` |
| `ShortAgentOnCall` | An agent handles the on-call over MCP | `out/short-agent-on-call.mp4` |
| `ShortLess` | Six services replaced by one binary | `out/short-less.mp4` |

```bash
npm run render:shorts                 # all four
npx remotion render src/index.ts ShortNothingLost out/short-nothing-lost.mp4 --codec h264 --crf 20
```

Post rules that worked for comparable launches: upload the MP4 natively, no
link in the tweet body, GitHub link in the first reply, one short per post.

## Films for X (30–46 s, 1920×1080)

`src/Sober.tsx` holds the two launch cuts in the "product on screen" grammar:
a serif title card, a real terminal whose outputs were captured on a live
instance (fake Resend answering 429 once, fake Telegram), one serif caption per
shot, an end card with the command. `SoberMix` (34 s, dark, terminal first then a
diagram: the launch cut), `SoberTerminal` (49 s, dark) and `SoberDiagram`
(38 s, light, a diagram that builds itself). Fonts in
`public/fonts` (Source Serif 4, JetBrains Mono, Inter, all OFL).
`npm run render:sober`.

`src/Clear.tsx` is the earlier launch cut (36 s, black on white, yellow highlighter):
who it is for, the four pillars (self-hosted, agent-native, open source,
Rust), what changes in six struck-through lines, the agent, the setup command.
`npm run render:clear` → `out/film-clear.mp4`.

Earlier dark cuts live in `src/Films.tsx`. `FilmThreeWays` (48 s) names the
alternatives:
who it is for, hosted vs self-hosted Novu vs notifyd (Novu's composition is
taken from their docker-compose documentation), what changes with each number
translated into a benefit, the agent, who it is for again. `FilmOne` (three
chapters, 42 s), `FilmTalk` (a chat with the agent, live counters, 46 s) and
`FilmNumbers` (before/after, 33 s) are earlier cuts.

```bash
npm run render:films                       # out/film-one.mp4, film-talk.mp4, film-numbers.mp4
REMOTION_LOGO=n-dot npm run render:films   # same films with another mark from public/logos/
```

`public/logos/` holds the chosen mark (`nd-two`, with and without the small
"rs"); it is the same drawing as `docs/assets/notifyd-logo.svg` and
`ui.tsx::Logo`. Change all three together.
