// Applied synchronously BEFORE styles.css paints, so the stored / system
// theme is correct on the first frame (no dark flash when the user has
// chosen light, and no light flash when they haven't). Kept as an external
// file so the app's script-src CSP can stay strict ('self' only) rather
// than having to allow 'unsafe-inline' for this one-liner. Loaded from
// index.html with the regular <script src>, which still blocks parsing.
(function () {
  try {
    var t = localStorage.getItem("sessionatlas.theme");
    if (t !== "light" && t !== "dark") {
      t = (window.matchMedia && window.matchMedia("(prefers-color-scheme: light)").matches) ? "light" : "dark";
    }
    document.documentElement.dataset.theme = t;
  } catch (e) { /* default = dark via :root */ }
})();
