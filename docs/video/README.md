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

Three longer cuts in `src/Films.tsx` that mix the "one call", "your agent on
call" and "less to run" angles, efficiency first: `FilmOne` (three chapters,
42 s), `FilmTalk` (the whole film is a chat with the agent, live counters on
top, 46 s), `FilmNumbers` (before/after split screen, 33 s).

```bash
npm run render:films                       # out/film-one.mp4, film-talk.mp4, film-numbers.mp4
REMOTION_LOGO=n-dot npm run render:films   # same films with another mark from public/logos/
```

`public/logos/` holds the logo candidates (badge, prompt, n-dot, mail-badge,
each as dark tile, light tile and bare mark); `REMOTION_LOGO` picks one
(default `badge`). The chosen mark becomes `docs/assets/notifyd-logo.svg`.
