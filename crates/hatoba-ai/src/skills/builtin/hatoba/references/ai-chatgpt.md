# Sign in with ChatGPT

A provider can sign in with a ChatGPT account instead of an API key. Its requests then use that account's ChatGPT plan, within the plan's limits. Other providers are in `references/ai-settings.md`.

| en | zh-CN | ja |
|---|---|---|
| Continue with ChatGPT | 使用 ChatGPT 继续 | ChatGPT で続行 |
| ChatGPT Account | ChatGPT 账户 | ChatGPT アカウント |
| Account | 账户 | アカウント |
| Sign Out | 退出登录 | サインアウト |
| Manage Usage | 管理用量 | 使用量を管理 |
| Continue in Your Browser | 在浏览器中继续 | ブラウザで続行 |
| You’re Using Your ChatGPT Plan | 你正在使用 ChatGPT 套餐 | ChatGPT プランを使用しています |
| Signed in on this device | 已在此设备登录 | このデバイスでサインイン済み |
| Sign in on this device | 需要在此设备登录 | このデバイスでサインインが必要 |
| Sign-in ended | 登录已失效 | サインインの期限切れ |
| Plan use not allowed | 未允许使用套餐 | プランの使用が未許可 |

## Signing in

- **Continue with ChatGPT** in Settings → AI → Providers (next to **Add Provider**, or in the empty state) opens OpenAI's sign-in page in the system browser and shows "Continue in Your Browser" until it comes back. Sign in, allow Hatoba to use the plan, and the browser says it can be closed. A provider named ChatGPT then appears with the models the account may use.
- The first sign-in of an account shows "You’re Using Your ChatGPT Plan". **Manage Usage** opens ChatGPT's usage settings (chatgpt.com/settings/usage), where usage is shown and Hatoba's own limit can be set.
- A ChatGPT provider has no protocol, base URL or key: it uses OpenAI's API at `api.openai.com`. Its row reads "ChatGPT Account" with the account's email. Its editor shows the **Account**, where the sign-in stands on this device, **Continue with ChatGPT** to sign in again, **Sign Out** and **Manage Usage**. The name and models are edited like any provider's; **Fetch Models** lists the models the account may use.
- To use another ChatGPT account or workspace, use **Continue with ChatGPT** next to **Add Provider** again: each account gets its own provider. Signing in there with an account that has a provider already, such as one synced from another device, signs in to that provider.

## On several devices

The provider and its account sync; the sign-in does not. Each device keeps its own, encrypted in its vault, and renews it by itself while it is used. A device that got the provider through sync shows "Sign in on this device" until you choose **Continue with ChatGPT** in the provider's editor there; the browser then only asks which account.

## Signing out

**Sign Out** ends the sign-in on this device and asks OpenAI to end the session; deleting the provider does the same, on every device signed in to it once the deletion syncs there. The provider keeps its account for the next sign-in. To stop Hatoba from using the plan on every device, disconnect Hatoba in ChatGPT's settings.

## Problems

| en | zh-CN | ja |
|---|---|---|
| Couldn’t Sign In to ChatGPT | 无法登录 ChatGPT | ChatGPT にサインインできませんでした |
| ChatGPT didn’t accept the sign-in. Sign in again. | ChatGPT 没有接受这次登录。请重新登录。 | ChatGPT がサインインを受け付けませんでした。もう一度サインインしてください。 |
| Signed out on this device, but ChatGPT didn’t confirm ending the session. You can disconnect Hatoba in ChatGPT’s settings. | 已在此设备退出，但 ChatGPT 没有确认结束会话。你可以在 ChatGPT 的设置里断开 Hatoba。 | このデバイスではサインアウトしましたが、ChatGPT はセッションの終了を確認できませんでした。ChatGPT の設定で Hatoba の接続を解除できます。 |
| Signed in, but Hatoba isn’t allowed to use your ChatGPT plan. Sign in again and allow it. | 已登录，但没有允许 Hatoba 使用你的 ChatGPT 套餐。重新登录并允许后才能使用。 | サインイン済みですが、Hatoba が ChatGPT プランを使うことは許可されていません。もう一度サインインして許可してください。 |

These messages come from Hatoba's backend and are in English in every language:

- "This ChatGPT provider is not signed in on this device…": sign in on this device (above).
- "Your ChatGPT sign-in has ended…": the session expired after a long time unused, was signed out in ChatGPT, or Hatoba was disconnected there. Choose **Continue with ChatGPT** in the provider's editor.
- "…does not allow Hatoba to use your plan": the sign-in did not allow plan use. Sign in again and allow it, or use a provider with an API key.
- "You have reached a usage limit of your ChatGPT plan…": the plan's limit, or the limit set for Hatoba, is reached. Check it with **Manage Usage**, wait for it to reset, or switch to another provider.
- "Your ChatGPT account or workspace cannot use its plan in Hatoba": the account, workspace or region is not eligible. Use a provider with an API key, or ask the workspace admin.
- "…could not check your plan's usage right now" and "…information is unavailable right now": a temporary problem at OpenAI; Hatoba already tried twice. **Retry** later.
- "Requests with a ChatGPT plan do not support `…`": that feature is not available with a ChatGPT plan; the name says which.
- "ChatGPT did not accept the sign-in for this request…": if it keeps happening, sign in again.
- "The sign-in was cancelled in the browser.", "The sign-in timed out…" (after 15 minutes), "Hatoba could not open the browser for the sign-in.": choose **Continue with ChatGPT** again; a browser must be set as the system default.
- "You signed in with another ChatGPT account than this provider's…": in the browser, pick the account the provider shows, or add a provider for the other account.
- "ChatGPT's ID token is not valid: it has expired; check this device's clock.": set the clock right (automatic time), then sign in again.
- "Unlock the vault to use the ChatGPT provider.": the sign-in is kept in the vault, which must be unlocked.
