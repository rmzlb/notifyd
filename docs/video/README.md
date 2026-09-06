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
