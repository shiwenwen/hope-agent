// Set background immediately to prevent white flash during resize.
// Kept as an external script (instead of inline in index.html) so the CSP
// can use `script-src 'self'` without `'unsafe-inline'`.
(function () {
  var t = localStorage.getItem("theme-preference") || "auto"
  var dark =
    t === "dark" ||
    (t === "auto" && window.matchMedia("(prefers-color-scheme: dark)").matches)
  if (dark) document.documentElement.classList.add("dark")
  document.documentElement.style.backgroundColor = dark ? "#0f0f0f" : "#ffffff"
  document.documentElement.style.colorScheme = dark ? "dark" : "light"
})()
