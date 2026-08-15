// Applied synchronously BEFORE first paint, so <html lang> is correct on
// the first frame and i18n text doesn't flash in the wrong language. Kept
// as an external file (not inline) so the app's script-src CSP can stay
// strict 'self'-only. Mirrors theme-init.js.
//
// Priority: explicit user choice (localStorage) > system language
// (navigator.language, zh* → zh, otherwise en) > default en.
(function () {
  try {
    var l = localStorage.getItem("sessionatlas.lang");
    if (l !== "en" && l !== "zh") {
      var sys = (navigator.language || "en").toLowerCase();
      l = sys.indexOf("zh") === 0 ? "zh" : "en";
    }
    document.documentElement.lang = l;
  } catch (e) {
    document.documentElement.lang = "en";
  }
})();
