# dsh-hatoba

Lets [DeepSeek Harness](https://github.com/deepseek-ai/deepseek-harness) work on the SSH hosts saved in
[Hatoba](https://hatoba.fog.moe), an open-source desktop SSH client. It adds the hatoba-cli skill, which has the agent
run commands and copy files on those hosts with the `hatoba` command, signed in with the credentials Hatoba keeps.

## Install

```sh
dsh plugin --profile web add dsh-hatoba
```

If the harness is running, restart it.

## Requirements

- [Hatoba](https://hatoba.fog.moe/download/), open and unlocked.
- In Hatoba's **Settings → General**, **Use the hatoba command in terminals** and **Let other programs use saved
  hosts** turned on.

Each stable Hatoba release publishes this package with the
[hatoba-cli skill](https://github.com/scarletkc/Hatoba/tree/main/skills/hatoba-cli) of that release.
