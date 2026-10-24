// Open the Xpra surface of the session named in this page's path.
//
// The desktop UI has already minted a single-use preview bootstrap. It passes
// that limited capability in the URL fragment, which is never sent to the
// gateway in the HTTP request. This page erases the fragment from history and
// POSTs the bootstrap to the preview origin; it never sees the gateway bearer.
"use strict";

(function () {
  const status = document.getElementById("status");
  const open = document.getElementById("open");
  let session;
  try {
    session = JSON.parse(decodeURIComponent(location.hash.slice(1)));
  } catch {
    session = null;
  }
  history.replaceState(null, "", location.pathname);

  async function launch() {
    open.disabled = true;
    status.textContent = "Opening the app...";
    try {
      if (!session || typeof session.bootstrap_token !== "string" || typeof session.url !== "string") {
        throw new Error("open this app from Capsem");
      }
      const url = new URL(session.url);
      if (url.protocol !== "http:" || !url.hostname.endsWith(".localhost") || url.pathname !== "/_capsem/bootstrap") {
        throw new Error("invalid preview destination");
      }
      const form = document.createElement("form");
      form.method = "POST";
      form.action = url.href;
      const input = document.createElement("input");
      input.type = "hidden";
      input.name = "bootstrap_token";
      input.value = session.bootstrap_token;
      form.append(input);
      document.body.append(form);
      form.submit();
    } catch (error) {
      status.textContent = `Could not open the app: ${error.message}.`;
      open.disabled = false;
    }
  }

  open.addEventListener("click", launch);
  // A page another site opened waits for a click: it can open this tab, but
  // not open the app in it.
  if (document.body.dataset.auto === "true" && session) {
    launch();
  } else {
    status.textContent = "Open this session's app?";
  }
})();
