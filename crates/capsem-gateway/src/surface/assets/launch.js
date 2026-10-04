// Open the Xpra surface of the session named in this page's path.
//
// The page is the gateway's own origin, so it reads the gateway token the way
// the Capsem UI does, asks for a single-use preview bootstrap for the
// session's surface, and POSTs it to the surface's preview origin. That origin
// answers with its HttpOnly session cookie and the app's client. No token
// enters a URL.
"use strict";

(function () {
  const status = document.getElementById("status");
  const open = document.getElementById("open");
  const vm = decodeURIComponent(location.pathname.split("/")[2] || "");

  async function launch() {
    open.disabled = true;
    status.textContent = "Opening the app...";
    try {
      const tokenResponse = await fetch("/token", { cache: "no-store" });
      if (!tokenResponse.ok) {
        throw new Error(`the gateway did not answer (${tokenResponse.status})`);
      }
      const { token } = await tokenResponse.json();
      const response = await fetch(`/vms/${encodeURIComponent(vm)}/surface/session`, {
        method: "POST",
        headers: { Authorization: `Bearer ${token}` },
        cache: "no-store",
      });
      if (response.status === 404) {
        throw new Error("this session has no app to open, or the app is not running yet");
      }
      if (!response.ok) {
        throw new Error(`the gateway refused (${response.status})`);
      }
      const session = await response.json();
      const form = document.createElement("form");
      form.method = "POST";
      form.action = session.url;
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
  if (document.body.dataset.auto === "true") {
    launch();
  } else {
    status.textContent = "Open this session's app?";
  }
})();
