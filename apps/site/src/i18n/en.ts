/**
 * The English text of the site, which zh.ts and ja.ts translate. `meta.description` is the one-line
 * description of Hatoba that the README, package.json, and the repository share; the other languages
 * translate it, and every page's description and the structured data start from it.
 */
export const en = {
  meta: {
    description:
      "Open-source desktop SSH client with an AI assistant and end-to-end encrypted sync through your own Cloudflare account",
    tagline: "Open-source SSH client with an AI assistant",
    downloadTitle: "Download Hatoba",
    downloadLead: (version: string) => `Download Hatoba ${version} for Windows, Linux, and macOS.`,
  },
  nav: {
    home: "Hatoba home",
    features: "Features",
    download: "Download",
    docs: "Docs",
    github: "GitHub",
    language: "Language",
    skip: "Skip to content",
  },
  platforms: { windows: "Windows", linux: "Linux", macos: "macOS" },
  hero: {
    downloadFor: (platform: string) => `Download for ${platform}`,
    allDownloads: "All downloads",
    github: "View on GitHub",
    available: (version: string) => `Version ${version} for Windows, Linux, and macOS`,
    alt: "Hatoba with the AI assistant beside a terminal, after it found and fixed an nginx 502 error",
  },
  ai: {
    title: "An AI assistant beside every terminal",
    body: "Describe the problem in the panel next to your terminal. The assistant reads the screen, runs commands, and reads and edits files on the host to find the cause and fix it.",
    points: [
      "Each command waits for your approval, and you can edit it before it runs. In Auto mode, a model you choose approves the safe ones.",
      "Every file it changes shows a diff that you can revert.",
      "Type / for commands and skills, and @ to attach files from the host. Add your own skills and MCP servers.",
      "Use any model with an OpenAI-compatible or Anthropic API, including local ones.",
    ],
    alt: "The approval card for a command the assistant wants to run",
  },
  sync: {
    title: "Sync through your own Cloudflare account",
    body: "Hatoba deploys a Worker and a D1 database to your Cloudflare account, then keeps your hosts, keys, and AI conversations in sync between your devices. The free plan usually covers personal use.",
    points: [
      "End-to-end encrypted: data is encrypted on your device before it leaves, so the Worker and the database hold only ciphertext.",
      "No Hatoba server: sync runs entirely in your account.",
      "Optional: everything works offline without it.",
    ],
    alt: "The Cloud Sync page with the connection, the devices, and the encryption notice",
  },
  essentials: {
    title: "Everything you need from an SSH client",
    items: [
      {
        term: "Hosts",
        text: "Groups, tags, favorites, and fuzzy search. Import ~/.ssh/config, connect through ProxyJump or a SOCKS5 or HTTP proxy, or type ssh user@host to connect without saving a host.",
      },
      {
        term: "Dashboard",
        text: "The status, CPU, memory, disks, and network of every saved host on one page, read over SSH with nothing to install on the servers.",
      },
      {
        term: "Terminal",
        text: "Tabs, truecolor, CJK input methods, search, clickable links, and a confirmation before a multi-line paste.",
      },
      { term: "SFTP", text: "A file panel beside the terminal, with drag-and-drop upload and downloads that you can cancel." },
      { term: "Port forwarding", text: "Local forwards that can start with the connection." },
      {
        term: "Keys",
        text: "Passwords, Ed25519, ECDSA, and RSA keys, ssh-agent, and keyboard-interactive 2FA. Import OpenSSH, PEM, and PuTTY keys, or generate new ones.",
      },
    ],
    alt: "The dashboard with the CPU, memory, and disk usage of every host",
  },
  security: {
    title: "Encrypted on your device",
    body: "A master password unlocks your vault. Keys are derived from it with Argon2id, and every item is encrypted with AES-256-GCM before it is stored or synced. If you forget the master password and lose the recovery code, your data cannot be recovered.",
    openSource: "Hatoba is open source under the MIT license, so you can check how it handles your data.",
    link: "Read the security model",
  },
  cta: {
    title: "Get Hatoba",
    body: "Free and open source, for Windows 10 and 11, Linux on x86_64, and Apple Silicon Macs (experimental).",
  },
  footer: {
    releases: "Releases",
    report: "Report a problem",
    security: "Security policy",
    contributing: "Contributing",
    license: "Hatoba is released under the MIT license.",
  },
  download: {
    released: (version: string, date: string) => `Version ${version}, released ${date}.`,
    notes: "Release notes",
    preview: (version: string) => `A preview of version ${version} is out.`,
    previewLink: "See the preview",
    thisDevice: "For this device",
    requirements: {
      windows: "Windows 10 or 11 (x64).",
      linux: "x86_64. Sync keeps its credentials in the Secret Service, so it needs GNOME Keyring, KWallet, or another Secret Service provider.",
      macos: "Apple Silicon, macOS 13 or later. This build is experimental.",
    },
    files: {
      nsis: "Installer (.exe)",
      deb: "Debian and Ubuntu (.deb)",
      appimage: "Other distributions (AppImage)",
      dmg: "Disk image (.dmg)",
    } as Record<string, string>,
    size: (megabytes: string) => `${megabytes} MB`,
    install: "Install",
    sha256: "SHA-256",
    copy: "Copy",
    copied: "Copied",
    checksums: "lists the checksum of every file in the release.",
    allReleases: "All releases",
    source: "Build from source",
    testing: "Hatoba is developed on Windows, so the Linux and macOS builds get less testing. If something doesn't work, please open an issue.",
    issue: "Open an issue",
  },
  notFound: {
    title: "Page not found",
    body: "There is no page at this address.",
    home: "Go to the home page",
  },
};

export type Messages = typeof en;
