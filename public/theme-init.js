// Set background immediately to prevent white flash during resize.
// Kept as an external script (instead of inline in index.html) so the CSP
// can use `script-src 'self'` without `'unsafe-inline'`.
(function () {
  var t = localStorage.getItem("theme-preference") || "auto"
  var dark =
    t === "dark" ||
    (t === "auto" && window.matchMedia("(prefers-color-scheme: dark)").matches)
  var transparentWindow =
    new URLSearchParams(window.location.search).get("window") === "pet"
  if (dark) document.documentElement.classList.add("dark")
  // PetWindow is a transparent overlay. Seeding its root canvas with the
  // normal opaque theme color would override the later `bg-transparent`
  // utility because inline styles win the cascade.
  document.documentElement.style.backgroundColor = transparentWindow
    ? "transparent"
    : dark
      ? "#0f0f0f"
      : "#ffffff"
  document.documentElement.style.colorScheme = dark ? "dark" : "light"
})()
