export {
  CapsemElement,
  CapsemElt,
  defineCapsemElement,
  type CapsemElementCleanup,
  type CapsemElementEventDetail,
  type CapsemElementRenderContext,
  type CapsemElementSpec,
} from "./capsem-elt";

import { defineCapsemElement } from "./capsem-elt";
import { CapsemArtifactElement } from "./capsem-artifacts";

export function defineCapsemElements(): void {
  if (typeof customElements === "undefined") return;
  defineCapsemElement();
  defineCapsemElement("capsem-sheet", class CapsemSheetElement extends CapsemArtifactElement {});
  defineCapsemElement("capsem-chart", class CapsemChartElement extends CapsemArtifactElement {});
  defineCapsemElement("capsem-diagram", class CapsemDiagramElement extends CapsemArtifactElement {});
  defineCapsemElement("capsem-text", class CapsemTextElement extends CapsemArtifactElement {});
  defineCapsemElement("capsem-media", class CapsemMediaElement extends CapsemArtifactElement {});
  defineCapsemElement("capsem-slide", class CapsemSlideElement extends CapsemArtifactElement {});
  defineCapsemElement(
    "capsem-slide-deck",
    class CapsemSlideDeckElement extends CapsemArtifactElement {},
  );
}
