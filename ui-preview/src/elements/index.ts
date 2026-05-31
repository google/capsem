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

export function defineCapsemElements(): void {
  if (typeof customElements === "undefined") return;
  defineCapsemElement();
}
