// Points the download button at the newest release's setup program, straight from GitHub.
// If there's no release yet, or GitHub can't be reached, the button opens the releases page.
(() => {
  "use strict";

  const REPO = "Thisisily/momputer";
  const RELEASES = `https://github.com/${REPO}/releases`;
  const API = `https://api.github.com/repos/${REPO}/releases/latest`;

  const $ = (id) => document.getElementById(id);
  const el = (tag, props = {}, ...kids) => {
    const node = Object.assign(document.createElement(tag), props);
    node.append(...kids);
    return node;
  };
  const sep = () => el("span", { className: "sep", ariaHidden: "true", textContent: "·" });
  const size = (bytes) => `${(bytes / 1048576).toFixed(1)} MB`;

  const dl = $("dl");
  const led = $("dl-led");
  const label = $("dl-label");
  const meta = $("dl-meta");
  const alt = $("dl-alt");

  // The page's other download buttons (the top bar, the first install step) go where the main one goes.
  const follow = () => {
    for (const a of document.querySelectorAll("a[data-dl]")) {
      a.href = dl.href;
      const light = a.querySelector(".led");
      if (light) light.className = led.className;
      const words = a.querySelector("[data-dl-label]");
      if (words) words.textContent = label.textContent;
    }
  };

  // Phones, Macs and Linux get a gentle note: momputer is for Windows.
  const platform = (navigator.userAgentData && navigator.userAgentData.platform) || navigator.platform || "";
  if (!/^win/i.test(platform) && !/Windows NT/i.test(navigator.userAgent)) $("not-windows").hidden = false;

  function ready(release, main, msi, sums) {
    const version = String(release.tag_name || "").replace(/^v/i, "");
    dl.href = main.browser_download_url;
    led.className = "led green";
    label.textContent = "Download for Windows";
    meta.replaceChildren(
      el("span", { textContent: version ? `Version ${version}` : release.name || "Latest release" }),
      sep(),
      el("span", { textContent: size(main.size) }),
      sep(),
      el("span", { textContent: "Windows 10 and 11, 64-bit" }),
    );
    if (msi && msi !== main) {
      alt.replaceChildren(
        "Prefer an MSI? ",
        el("a", { href: msi.browser_download_url, textContent: "Download the .msi" }),
        ` (${size(msi.size)})`,
      );
      alt.hidden = false;
    }

    // Checking the download: the exact file name, and the fingerprint GitHub lists for it.
    $("sums").href = sums ? sums.browser_download_url : release.html_url || RELEASES;
    const file = `"$HOME\\Downloads\\${main.name}"`;
    const digest = /^sha256:([0-9a-f]{64})$/i.exec(main.digest || "");
    if (digest) {
      $("hash-cmd").textContent = `(Get-FileHash ${file}).Hash -eq '${digest[1].toUpperCase()}'`;
      $("hash-says").replaceChildren(
        "It should print ",
        el("b", { textContent: "True" }),
        ". The long code is the fingerprint GitHub lists for ",
        el("code", { textContent: main.name }),
        ", and the same one is in ",
        el("code", { textContent: "SHA256SUMS.txt" }),
        ".",
      );
    } else {
      $("hash-cmd").textContent = `Get-FileHash ${file}`;
    }
    follow();
  }

  // No release published yet (or one without an installer): send people to the releases page.
  function notYet(url) {
    dl.href = url || RELEASES;
    led.className = "led";
    label.textContent = "See releases on GitHub";
    meta.replaceChildren("No release yet. The first one is on its way.");
    alt.replaceChildren(
      "Can't wait? ",
      el("a", { href: `https://github.com/${REPO}#install`, textContent: "Build it from source" }),
      ".",
    );
    alt.hidden = false;
    follow();
  }

  // GitHub didn't answer (offline, blocked, or too many questions this hour).
  function unreachable() {
    dl.href = `${RELEASES}/latest`;
    led.className = "led";
    label.textContent = "Download from GitHub";
    meta.replaceChildren("Couldn't ask GitHub for the newest version, so this opens its releases page. Pick the file ending in -setup.exe.");
    follow();
  }

  const abort = new AbortController();
  const timer = setTimeout(() => abort.abort(), 8000);
  fetch(API, { headers: { Accept: "application/vnd.github+json" }, signal: abort.signal })
    .then((res) => {
      if (res.status === 404) return null;
      if (!res.ok) throw new Error(`GitHub said ${res.status}`);
      return res.json();
    })
    .then((release) => {
      if (!release) return notYet();
      const assets = Array.isArray(release.assets) ? release.assets : [];
      const exe = assets.find((a) => /-setup\.exe$/i.test(a.name));
      const msi = assets.find((a) => /\.msi$/i.test(a.name));
      const sums = assets.find((a) => /^SHA256SUMS\.txt$/i.test(a.name));
      const main = exe || msi;
      if (!main) return notYet(release.html_url);
      ready(release, main, msi, sums);
    })
    .catch(unreachable)
    .finally(() => clearTimeout(timer));

  // Copy buttons.
  for (const button of document.querySelectorAll("button.copy")) {
    button.addEventListener("click", async () => {
      const target = $(button.dataset.for);
      try {
        await navigator.clipboard.writeText(target.textContent);
        button.textContent = "Copied";
      } catch {
        // No clipboard access: select it so Ctrl+C works.
        const range = document.createRange();
        range.selectNodeContents(target);
        const selection = getSelection();
        selection.removeAllRanges();
        selection.addRange(range);
        button.textContent = "Press Ctrl+C";
      }
      clearTimeout(button.timer);
      button.timer = setTimeout(() => (button.textContent = "Copy"), 1800);
    });
  }
})();
