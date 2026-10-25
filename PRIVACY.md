# Privacy policy

Hatoba is an open-source desktop SSH client, made by ScarletKc. It has no telemetry, no analytics, no crash reporting, and no ads, and it sends nothing about you to its developer. What leaves your device goes to the servers and services you connect Hatoba to yourself, as described below.

This policy covers the Hatoba desktop app, however you installed it, and the website at [hatoba.fog.moe](https://hatoba.fog.moe).

## On your device

- **The vault.** Your hosts, keys, passwords, settings, and AI conversations are kept in an encrypted vault in Hatoba's data folder.
- **The system's credential store.** Some credentials, such as the ones sync uses and the encrypted vault key that Windows Hello unlocks, are kept in Windows Credential Manager, the macOS Keychain, or the Secret Service on Linux.
- **Logs.** Hatoba writes local logs that name the SSH servers it connected to. A new log file starts each day, and Hatoba keeps the 7 newest. They stay on your device unless you share them yourself.

## What leaves your device

**SSH connections.** Terminals, file transfers, and port forwarding talk to the servers you connect to, over SSH.

**Sync, if you turn it on.** Sync stores your vault in a D1 database in your own Cloudflare account, either through a Worker that you deploy there or, in D1 direct mode, through the Cloudflare API with an API token you give Hatoba. Everything is encrypted on your device before it is uploaded, so the server never sees your passwords, keys, or any other item's contents. It does see the number of items, their IDs, their encrypted sizes and modification times, the number of devices, sign-in and sync times, and the IP addresses that Cloudflare sees for any request. Deploying the Worker calls the Cloudflare API with the API token you give Hatoba.

**The AI assistant, if you set it up.** The assistant sends each request to the AI provider you add, or to OpenAI when you sign in with ChatGPT. A request carries:

- The conversation: your messages, the files and text you attach, and the results of the tools the assistant runs, such as command output, file contents, search results, and web pages.
- The screen of the terminal tab you attached, with each message you send, while **Send Terminal Screen** is on.
- Context that Hatoba adds: the host's display name and user name and the version its SSH server reports (or, for a local terminal, the computer's name, system, shell, and user name), your custom instructions, the host's AI notes, and the assistant's memories if memory is on.

Addresses, passwords, keys, and other hosts reach the provider only when they appear on the screen or in a tool result, or when you attach them. The provider handles what it receives under its own privacy policy.

In auto mode, the judge you choose in Settings, a model of a provider you added or a System One endpoint, also receives requests: your messages, your custom instructions and the host's AI notes, and each tool call it decides on with its arguments, including the terminal's screen lines when the call types into a terminal.

When you set them up, the assistant also sends web searches to the search service you choose (Brave Search, Tavily, or a SearXNG instance) or uses the provider's own search, requests the web pages it reads from their sites, and sends its calls to the MCP servers you add.

**Model data.** When a provider's model list leaves out a model's limits, Hatoba downloads the public model data at models.dev. The request carries nothing about you.

**Update checks.** Hatoba asks GitHub for the latest release after the first unlock each time it starts and once a day while it runs, and downloads updates from GitHub. The check sends no identifiers. You can turn the automatic check off on the About page in Settings. A copy installed through a package manager leaves updates to the package manager and doesn't check.

**Reporting a problem.** **Report a Problem** opens a GitHub issue form in your browser. The app version and your operating system are in the form's link, so GitHub receives them when the form opens. What you write is sent only when you submit the form yourself.

Every service above sees your IP address and handles the request under its own privacy policy.

## The website

hatoba.fog.moe is a static site served by GitHub Pages through Cloudflare. It sets no cookies and has no analytics or tracking. When you pick a language, your browser remembers it in local storage, and nothing is sent anywhere. GitHub and Cloudflare see the requests your browser makes, as they would for any site they serve.

## Changes

This policy changes with Hatoba. Every change is in [the history of this file](https://github.com/scarletkc/Hatoba/commits/main/PRIVACY.md).

## Contact

Ask a question or report a problem with this policy in [an issue](https://github.com/scarletkc/Hatoba/issues). For a security vulnerability, follow the [security policy](SECURITY.md).
