# hatoba-agent-plugin

Lets coding agents work on the SSH hosts saved in [Hatoba](https://hatoba.fog.moe), an open-source desktop SSH client.
It adds the [hatoba-cli skill](https://github.com/scarletkc/Hatoba/tree/main/skills/hatoba-cli), which has the agent run
commands and copy files on those hosts with the `hatoba` command, signed in with the credentials Hatoba keeps. Each
stable Hatoba release publishes this package with the skill of that release.

## Requirements

- [Hatoba](https://hatoba.fog.moe/download/), open and unlocked.
- In Hatoba's **Settings → General**, **Use the hatoba command in terminals** and **Let other programs use saved
  hosts** turned on.

## Claude Code

In Claude Code, add Hatoba's marketplace and install the plugin:

```
/plugin marketplace add https://raw.githubusercontent.com/scarletkc/Hatoba/main/.claude-plugin/marketplace.json
/plugin install hatoba@hatoba-marketplace
```

## Codex

```sh
codex plugin marketplace add scarletkc/Hatoba --sparse .agents/plugins
codex plugin add hatoba@hatoba-marketplace
```

## DeepSeek Harness

Install the plugin into the [DeepSeek Harness](https://github.com/deepseek-ai/deepseek-harness) profile you run, then
restart the harness if it is running. The desktop app runs the `desktop` profile, and `dsh web` runs `web`:

```sh
# For the desktop app
dsh plugin --profile desktop add hatoba-agent-plugin
# For dsh web
dsh plugin --profile web add hatoba-agent-plugin
```

To update, add the latest version to the same profile, then restart the harness:

```sh
dsh plugin --profile desktop add hatoba-agent-plugin@latest
```

`update` in place of `add` stays within the version range saved at install, so it doesn't reach a new minor version.
