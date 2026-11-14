import { defineConfig } from "vitepress";

/**
 * DBPinball documentation site.
 *
 * The content is sourced from the repository: the CLI definitions in
 * `crates/dbpinball-cli/src/cli.rs`, the store in
 * `crates/dbpinball-core/src/store/`, the Tauri commands in
 * `src-tauri/src/lib.rs`, and the tests that pin their behaviour.
 */
export default defineConfig({
  title: "DBPinball",
  description:
    "Documentation for DBPinball, a CLI-first database workspace for MySQL and PostgreSQL.",
  lang: "en-US",
  cleanUrls: true,
  lastUpdated: true,
  // Explicit Shiki themes: light tokens on a light code background, dark tokens
  // on a dark one. The code-block background is set in the theme CSS and must
  // match the mode, or dark tokens land on a dark background.
  markdown: {
    theme: {
      light: "github-light-high-contrast",
      dark: "github-dark-high-contrast",
    },
  },
  // The repository root has a Tailwind PostCSS config for the desktop app.
  // VitePress must not inherit it: the docs site has its own CSS and CI does not
  // install the root node_modules. An explicit empty plugin list stops Vite from
  // searching for a config file.
  vite: {
    css: {
      postcss: { plugins: [] },
    },
  },
  head: [
    ["meta", { name: "theme-color", content: "#2563eb" }],
    ["link", { rel: "icon", href: "/favicon.svg", type: "image/svg+xml" }],
  ],
  themeConfig: {
    logo: "/logo.svg",
    siteTitle: "DBPinball",
    nav: [
      { text: "Guide", link: "/guide/", activeMatch: "/guide/" },
      { text: "Architecture", link: "/architecture/", activeMatch: "/architecture/" },
      { text: "Security", link: "/security/", activeMatch: "/security/" },
      { text: "Reference", link: "/reference/cli", activeMatch: "/reference/" },
      { text: "Roadmap", link: "/roadmap" },
    ],
    sidebar: {
      "/guide/": [
        {
          text: "Getting Started",
          items: [
            { text: "Overview", link: "/guide/" },
            { text: "Requirements", link: "/guide/requirements" },
            { text: "Installation", link: "/guide/installation" },
            { text: "Quick start", link: "/guide/quick-start" },
            { text: "First connection", link: "/guide/first-connection" },
          ],
        },
        {
          text: "Using DBPinball",
          items: [
            { text: "Managing connections", link: "/guide/connections" },
            { text: "Credentials and keyring", link: "/guide/credentials" },
            { text: "CLI basics", link: "/guide/cli" },
            { text: "Desktop basics", link: "/guide/desktop" },
            { text: "Configuration", link: "/guide/configuration" },
            { text: "Troubleshooting", link: "/guide/troubleshooting" },
          ],
        },
      ],
      "/architecture/": [
        {
          text: "Architecture",
          items: [
            { text: "System overview", link: "/architecture/" },
            { text: "Rust core", link: "/architecture/core" },
            { text: "Tauri desktop", link: "/architecture/desktop" },
            { text: "CLI", link: "/architecture/cli" },
            { text: "Laravel adapter", link: "/architecture/laravel-adapter" },
            { text: "Connection store", link: "/architecture/connection-store" },
            { text: "Revision consistency", link: "/architecture/revision-consistency" },
            { text: "Credential lifecycle", link: "/architecture/credential-lifecycle" },
            { text: "Adapter boundaries", link: "/architecture/adapter-boundaries" },
          ],
        },
      ],
      "/security/": [
        {
          text: "Security",
          items: [
            { text: "Overview", link: "/security/" },
            { text: "Security policy", link: "/security/policy" },
            { text: "Destination verification", link: "/security/destination-verification" },
            { text: "Threat model", link: "/security/threat-model" },
            { text: "Safe Sync design (planned)", link: "/security/safe-sync-design" },
            { text: "Credential storage", link: "/security/credential-storage" },
            { text: "No plaintext fallback", link: "/security/no-plaintext-fallback" },
            { text: "OS keyring integration", link: "/security/keyring" },
            { text: "Redaction guarantees", link: "/security/redaction" },
            { text: "Concurrent mutation safety", link: "/security/concurrency" },
            { text: "Limitations", link: "/security/limitations" },
            { text: "Platforms", link: "/security/platforms" },
          ],
        },
      ],
      "/reference/": [
        {
          text: "Reference",
          items: [
            { text: "CLI commands", link: "/reference/cli" },
            { text: "Security CLI", link: "/reference/security-cli" },
            { text: "Configuration", link: "/reference/configuration" },
            { text: "Error behaviour", link: "/reference/errors" },
            { text: "Profile schema", link: "/reference/profile-schema" },
            { text: "Developer troubleshooting", link: "/reference/troubleshooting" },
          ],
        },
      ],
      "/development/": [
        {
          text: "Development",
          items: [
            { text: "Local setup", link: "/development/" },
            { text: "Repository structure", link: "/development/repository-structure" },
            { text: "Testing", link: "/development/testing" },
            { text: "CI checks", link: "/development/ci" },
            { text: "Contributing", link: "/development/contributing" },
            { text: "Pull request workflow", link: "/development/pull-requests" },
          ],
        },
      ],
    },
    socialLinks: [
      { icon: "github", link: "https://github.com/programinglive/dbpinball" },
    ],
    search: {
      provider: "local",
      options: {
        detailedView: true,
      },
    },
    editLink: {
      pattern:
        "https://github.com/programinglive/dbpinball/edit/main/website/:path",
      text: "Edit this page on GitHub",
    },
    footer: {
      message: "Released under the MIT License.",
      copyright: "DBPinball",
    },
    outline: { level: [2, 3], label: "On this page" },
    docFooter: { prev: "Previous", next: "Next" },
    darkModeSwitchLabel: "Appearance",
    sidebarMenuLabel: "Menu",
    returnToTopLabel: "Return to top",
  },
});
