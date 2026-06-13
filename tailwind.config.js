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
      // Colours reference the CSS custom properties from static/tokens.css so
      // the `bg-pl-*`/`text-pl-*`/`border-pl-*` utilities follow the active
      // theme (dark by default, light under `<html class="light">`).
      colors: {
        "pl-bg": "var(--pl-bg)",
        "pl-bg-2": "var(--pl-bg-2)",
        "pl-bg-3": "var(--pl-bg-3)",
        "pl-line": "var(--pl-line)",
        "pl-line-2": "var(--pl-line-2)",
        "pl-fg": "var(--pl-fg)",
        "pl-fg-2": "var(--pl-fg-2)",
        "pl-mute": "var(--pl-mute)",
        "pl-mute-2": "var(--pl-mute-2)",
        "pl-green": "var(--pl-green)",
        "pl-yellow": "var(--pl-yellow)",
        "pl-red": "var(--pl-red)",
        "pl-blue": "var(--pl-blue)",
      },
      fontFamily: {
        display: ['"Archivo Black"', "sans-serif"],
        mono: ['"JetBrains Mono"', "ui-monospace", "monospace"],
        body: ["Inter", "system-ui", "sans-serif"],
      },
    },
  },
};
