import React from "react";
import { Composition } from "remotion";
import { Explainer, TOTAL_FRAMES } from "./Explainer";
import { SHORT_FRAMES, ShortAgentOnCall, ShortLess, ShortNothingLost, ShortOneCall } from "./Shorts";

export const Root: React.FC = () => (
  <>
    <Composition id="Explainer" component={Explainer} durationInFrames={TOTAL_FRAMES} fps={30} width={1280} height={720} />
    <Composition id="ShortOneCall" component={ShortOneCall} durationInFrames={SHORT_FRAMES} fps={30} width={1920} height={1080} />
    <Composition id="ShortNothingLost" component={ShortNothingLost} durationInFrames={SHORT_FRAMES} fps={30} width={1920} height={1080} />
    <Composition id="ShortAgentOnCall" component={ShortAgentOnCall} durationInFrames={SHORT_FRAMES} fps={30} width={1920} height={1080} />
    <Composition id="ShortLess" component={ShortLess} durationInFrames={SHORT_FRAMES} fps={30} width={1920} height={1080} />
  </>
);
