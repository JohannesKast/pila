/** @type {import('tailwindcss').Config} */
// Production Tailwind build config. Mirrors the former inline `tailwind.config`
// that the Play CDN used (see git history of templates/base.html). The CSS is
// compiled at build time into static/app.css — the CDN is no longer loaded.
module.exports = {
  // Scan everywhere a utility class can appear. Note `src/**/*.rs`: some
  // handlers emit HTML fragments with literal Tailwind classes (e.g.
  // `text-emerald-400` in admin.rs), which would otherwise be purged.
  content: ["./templates/**/*.html", "./src/**/*.rs"],
  darkMode: "class",
  theme: {
    extend: {
      colors: {
        "pl-bg": "#0a0f0c",
        "pl-bg-2": "#131a16",
        "pl-bg-3": "#1c2521",
        "pl-line": "#283130",
        "pl-line-2": "#3a4541",
        "pl-fg": "#f3f3ed",
        "pl-fg-2": "#cbd1cc",
        "pl-mute": "#7c8783",
        "pl-mute-2": "#4d5754",
        "pl-green": "#74ff8c",
        "pl-yellow": "#ffe600",
        "pl-red": "#ff4d4d",
        "pl-blue": "#5fb7ff",
      },
      fontFamily: {
        display: ['"Archivo Black"', "sans-serif"],
        mono: ['"JetBrains Mono"', "ui-monospace", "monospace"],
        body: ["Inter", "system-ui", "sans-serif"],
      },
    },
  },
};
