# hatoba-agent-plugin

Lets coding agents work on the SSH hosts saved in [Hatoba](https://hatoba.fog.moe), an open-source desktop SSH client.
It adds the [hatoba-cli skill](https://github.com/scarletkc/Hatoba/tree/main/skills/hatoba-cli), which has the agent run
commands and copy files on those hosts with the `hatoba` command, signed in with the credentials Hatoba keeps. Each
stable Hatoba release publishes this package with the skill of that release.

## Requirements

- [Hatoba](https://hatoba.fog.moe/download/), open and unlocked.
- In Hatoba's **Settings → General**, **Use the hatoba command in terminals** and **Let other programs use saved
  hosts** turned on.

## DeepSeek Harness

Install the plugin in [DeepSeek Harness](https://github.com/deepseek-ai/deepseek-harness), then restart the harness if
it is running:

```sh
dsh plugin --profile web add hatoba-agent-plugin
```

To update, add the latest version, then restart the harness:

```sh
dsh plugin --profile web add hatoba-agent-plugin@latest
```

`dsh plugin --profile web update` stays within the version range saved at install, so it doesn't reach a new minor
version.
