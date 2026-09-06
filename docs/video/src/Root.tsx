import React from "react";
import { Composition } from "remotion";
import { Explainer, TOTAL_FRAMES } from "./Explainer";

export const Root: React.FC = () => (
  <Composition id="Explainer" component={Explainer} durationInFrames={TOTAL_FRAMES} fps={30} width={1280} height={720} />
);
