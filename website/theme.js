// Runs in <head> before first paint so the page never flashes the wrong theme.
// Preference: "system" (default), "dark" or "light", remembered in localStorage.
(function () {
  var pref = "system";
  try {
    pref = localStorage.getItem("kw-theme") || "system";
  } catch (e) {}
  if (pref !== "dark" && pref !== "light") pref = "system";
  var dark =
    pref === "dark" ||
    (pref === "system" && window.matchMedia && matchMedia("(prefers-color-scheme: dark)").matches);
  var root = document.documentElement;
  root.setAttribute("data-theme", dark ? "dark" : "light");
  root.setAttribute("data-theme-pref", pref);
})();
