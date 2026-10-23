// The desktop mints a one-use preview capability. Its fragment is never sent
// in the HTTP request; erase it before submitting the scoped bootstrap form.
// This page never receives the gateway bearer.
"use strict";

(function () {
  const status = document.getElementById("status");
  const open = document.getElementById("open");
  const progress = document.getElementById("progress");
  const parentOrigin = document.body.dataset.parent;
  let session;
  try {
    session = JSON.parse(decodeURIComponent(location.hash.slice(1)));
  } catch {
    session = null;
  }
  history.replaceState(null, "", location.pathname + location.search);

  function launch() {
    open.disabled = true;
    open.hidden = true;
    progress.hidden = false;
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
      if (parentOrigin) {
        url.pathname = "/_capsem/surface/index.html";
        form.action = url.href;
        for (const [name, value] of [["parent_origin", parentOrigin], ["display_profile", document.body.dataset.profile || "auto"]]) {
          const input = document.createElement("input");
          input.type = "hidden";
          input.name = name;
          input.value = value;
          form.append(input);
        }
      }
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
      open.hidden = false;
      progress.hidden = true;
    }
  }

  open.addEventListener("click", launch);
  if (document.body.dataset.auto === "true" && session) {
    launch();
  } else {
    progress.hidden = true;
    status.textContent = "Open this session's app?";
  }
})();
