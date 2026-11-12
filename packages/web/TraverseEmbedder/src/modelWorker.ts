/**
 * Spec 138 browser guest Worker (#1582, Decision 104). `ExactModelBrowserHost`
 * runs each package's guest here so cancellation and deadlines can terminate
 * a running inference. One Worker per package holds its compiled module and
 * its ABI v3 snapshot; requests arrive one at a time.
 */

import { createModelGuestHandler, type ModelGuestRequest } from "./exactModel.js";

const handle = createModelGuestHandler();
const scope = globalThis as unknown as {
  onmessage: ((event: { readonly data: { id: number; request: ModelGuestRequest } }) => void) | null;
  postMessage(message: unknown): void;
};

scope.onmessage = (event) => {
  const { id, request } = event.data;
  void handle(request).then((reply) => scope.postMessage({ id, reply }));
};
