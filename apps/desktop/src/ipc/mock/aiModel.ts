import { parseMessage } from "@/lib/ai/attachments";
import type { AiEntryView, AiSideQuestionInput, AiToolCall } from "../types";
import {
  CUT_SHORT,
  DISK_COMMAND,
  lastUser,
  MAINTENANCE,
  type AiMockCtx,
  type Conv,
  type Plan,
  type ToolEntry,
  type Turn,
} from "./aiShared";
import { VISITOR } from "./visitor";

export function createAiModel(ctx: AiMockCtx) {
  const { deps, flags, say, sayZh, turns, call } = ctx;

  /** AI-42: the fake model's answer to a side question, about the call whose approval card waits or the conversation. */
  function sideAnswer(c: Conv, input: AiSideQuestionInput): string {
    const running = turns.has(c.view.id);
    const pending =
      running && input.approval_call_id
        ? c.entries
            .flatMap((e) => (e.role === "assistant" ? e.tool_calls : []))
            .find((x) => x.id === input.approval_call_id)
        : undefined;
    if (pending) {
      let args: Record<string, unknown> = {};
      try {
        args = JSON.parse(pending.arguments) as Record<string, unknown>;
      } catch {
        // cut-off arguments name nothing
      }
      const named = [args.command, args.path, args.url, args.text].find((x) => typeof x === "string") as
        string | undefined;
      const what = named ? `\`${named.split("\n")[0]}\`` : `\`${pending.name}\``;
      return sayZh(
        `This **${pending.name}** call works on ${what}. It is still waiting for your approval, so nothing has run yet.\n\n- **Run** lets it go ahead, and the task continues from its result.\n- **Reject** skips it, and you can tell the assistant why.\n\nThis answer is not saved in the conversation.`,
        `这个 **${pending.name}** 调用会处理 ${what}。它还在等你批准，所以目前什么都没有运行。\n\n- **运行**：让它继续，任务会接着用它的结果往下走。\n- **拒绝**：不运行它，可以顺便告诉助手原因。\n\n这条回答不会保存到对话里。`,
        `這個 **${pending.name}** 呼叫會處理 ${what}。它還在等你核准，所以目前什麼都沒有執行。\n\n- **執行**：讓它繼續，任務會接著用它的結果往下走。\n- **拒絕**：不執行它，可以順便告訴助理原因。\n\n這條回答不會儲存到對話裡。`,
      );
    }
    const typed = parseMessage(input.text).typed.trim() || input.text.trim();
    const replies = c.entries.filter((e) => e.role === "assistant").length;
    return sayZh(
      `A quick answer to “${typed}”: going by the ${replies} replies so far, this conversation is about the host’s state, and ${running ? "the task in progress keeps going" : "nothing is running right now"}.\n\nThis answer is not saved in the conversation.`,
      `简单回答“${typed}”：从目前的 ${replies} 条回复看，这段对话在处理这台主机的状态，${running ? "当前任务仍在继续" : "现在没有任务在运行"}。\n\n这条回答不会保存到对话里。`,
      `簡單回答「${typed}」：從目前的 ${replies} 條回覆看，這段對話在處理這台主機的狀態，${running ? "目前的任務仍在繼續" : "現在沒有任務在執行"}。\n\n這條回答不會儲存到對話裡。`,
    );
  }

  /**
   * A call of a tool the conversation's MCP servers offer: a running server's, or one started for it
   * (as Rust starts a `stdio` server when a conversation first needs its tools, AI-32). A tool that
   * asks is preferred, to show the approval card. Without one, a name no server offers.
   */
  async function mcpCall(turn: Turn, words: string): Promise<AiToolCall> {
    const args = { query: "nginx 502 bad gateway", limit: 5 };
    const mcp = deps.mcp;
    if (mcp) {
      try {
        // A server named in the message goes first ("mcp filesystem").
        const named = (name: string) => (words.includes(name.toLowerCase()) ? 0 : 1);
        const servers = (await mcp.servers())
          .filter((s) => s.enabled && !turn.context.disabled_mcp_servers.includes(s.id))
          .sort((a, b) => named(a.name) - named(b.name));
        for (const s of servers) {
          let status = await mcp.status(s.id);
          if (status.state === "stopped") status = await mcp.start(s.id);
          const tool = status.tools.find((x) => !x.always_allow) ?? status.tools[0];
          if (status.state === "running" && tool) return call(tool.name, args);
        }
      } catch {
        // fall through to a tool no server offers
      }
    }
    return call("mcp__github__list_issues", { repo: "scarletkc/Hatoba", state: "open" });
  }

  /**
   * `?ai=showcase`: step `step` (the replies so far in this turn) of the scripted nginx 502 fix. It is
   * written in Japanese too, since the website's Japanese screenshots show it.
   */
  function showcase(step: number): Plan {
    if (step === 0)
      return {
        reasoning: say(
          "A 502 means nginx could not get a response from the upstream. Check nginx’s error log, then the port the API actually listens on.",
          "502 说明 nginx 连不上上游。先看 nginx 的错误日志，再看 API 实际监听的端口。",
          "502 は nginx が上流から応答を受け取れていないということ。nginx のエラーログと、API が実際に待ち受けているポートを確認する。",
          "502 說明 nginx 連不上上游。先看 nginx 的錯誤日誌，再看 API 實際監聽的連接埠。",
        ),
        text: say(
          "Let me check nginx’s error log and the port the API listens on.",
          "我先看 nginx 的错误日志和 API 监听的端口。",
          "nginx のエラーログと、API が待ち受けているポートを確認します。",
          "我先看 nginx 的錯誤日誌和 API 監聽的連接埠。",
        ),
        calls: [
          call("run_command", { command: "sudo tail -n 3 /var/log/nginx/error.log", timeout_seconds: 30 }),
          call("run_command", { command: "sudo ss -ltnp | grep api", timeout_seconds: 30 }),
        ],
        finish: "tool_calls",
      };
    if (step === 1)
      return {
        reasoning: say(
          "nginx connects to 8081, but the API listens on 8080. Fix the upstream, test the config, then reload.",
          "nginx 连的是 8081，API 监听的是 8080。改 upstream，测试配置后重新加载。",
          "nginx は 8081 に接続しているが、API は 8080 で待ち受けている。upstream を直し、設定をテストしてからリロードする。",
          "nginx 連的是 8081，API 監聽的是 8080。改 upstream，測試設定後重新載入。",
        ),
        text: say(
          "Found it: nginx forwards to `127.0.0.1:8081`, but `api` listens on `127.0.0.1:8080`. I’ll point the upstream at 8080, test the config, and reload nginx.",
          "找到了：nginx 把请求转发到 `127.0.0.1:8081`，但 `api` 监听的是 `127.0.0.1:8080`。我把 upstream 改成 8080，测试配置后重新加载 nginx。",
          "原因がわかりました。nginx は `127.0.0.1:8081` に転送していますが、`api` は `127.0.0.1:8080` で待ち受けています。upstream を 8080 に変更し、設定をテストしてから nginx をリロードします。",
          "找到了：nginx 把請求轉送到 `127.0.0.1:8081`，但 `api` 監聽的是 `127.0.0.1:8080`。我把 upstream 改成 8080，測試設定後重新載入 nginx。",
        ),
        calls: [
          call("run_command", {
            command:
              "sudo sed -i 's/127.0.0.1:8081/127.0.0.1:8080/' /etc/nginx/conf.d/api.conf && sudo nginx -t && sudo systemctl reload nginx",
            timeout_seconds: 30,
          }),
        ],
        finish: "tool_calls",
      };
    if (step === 2)
      return {
        reasoning: "",
        text: say(
          "nginx reloaded. I’ll verify it from your shell.",
          "nginx 已重新加载。我在你的终端里验证一下。",
          "nginx をリロードしました。ターミナルで確認します。",
          "nginx 已重新載入。我在你的終端機裡驗證一下。",
        ),
        calls: [call("send_input", { text: "curl -sI http://localhost/v1/hosts", keys: ["enter"], wait_seconds: 3 })],
        finish: "tool_calls",
      };
    return {
      reasoning: "",
      text:
        say(
          "**Fixed.** The 502s came from a port mismatch:\n\n| Check | Before | Now |\n|---|---|---|\n| nginx upstream | `127.0.0.1:8081` | `127.0.0.1:8080` |\n| `GET /v1/hosts` | 502 Bad Gateway | **200 OK** |\n\nThe last deploy moved the API to port 8080, but `/etc/nginx/conf.d/api.conf` still pointed at 8081. A check at the end of the deploy script catches this next time:\n\n```bash\ncurl -fsS http://localhost/healthz || exit 1\n```",
          "**修好了。** 502 来自端口不一致：\n\n| 检查项 | 之前 | 现在 |\n|---|---|---|\n| nginx upstream | `127.0.0.1:8081` | `127.0.0.1:8080` |\n| `GET /v1/hosts` | 502 Bad Gateway | **200 OK** |\n\n上次部署把 API 改到了 8080 端口，但 `/etc/nginx/conf.d/api.conf` 还指向 8081。可以在部署脚本最后加一个检查，下次就能马上发现：\n\n```bash\ncurl -fsS http://localhost/healthz || exit 1\n```",
          "**修正しました。** 502 の原因はポートの不一致でした。\n\n| 確認項目 | 修正前 | 修正後 |\n|---|---|---|\n| nginx upstream | `127.0.0.1:8081` | `127.0.0.1:8080` |\n| `GET /v1/hosts` | 502 Bad Gateway | **200 OK** |\n\n前回のデプロイで API は 8080 に移りましたが、`/etc/nginx/conf.d/api.conf` は 8081 のままでした。デプロイスクリプトの最後にこのチェックを入れれば、次はすぐ気づけます。\n\n```bash\ncurl -fsS http://localhost/healthz || exit 1\n```",
          "**修好了。** 502 來自連接埠不一致：\n\n| 檢查項 | 之前 | 現在 |\n|---|---|---|\n| nginx upstream | `127.0.0.1:8081` | `127.0.0.1:8080` |\n| `GET /v1/hosts` | 502 Bad Gateway | **200 OK** |\n\n上次部署把 API 改到了 8080 連接埠，但 `/etc/nginx/conf.d/api.conf` 還指向 8081。可以在部署指令碼最後加一個檢查，下次就能馬上發現：\n\n```bash\ncurl -fsS http://localhost/healthz || exit 1\n```",
        ) +
        (VISITOR
          ? say(
              "\n\n---\n\nThat was the demo’s scripted fix. You can also ask how much **disk** space is left or what’s on the **screen**, or ask me to **type** a command, **search** the web, or **edit** the nginx config.",
              "\n\n---\n\n以上是演示里预先写好的修复过程。你还可以问我**磁盘**还剩多少空间、**屏幕**上有什么，或者让我**输入**一条命令、**搜索**网页、**编辑** nginx 配置。",
              "\n\n---\n\nここまではデモ用に用意した修正の流れです。ほかにも、**ディスク**の空きや**画面**の内容を聞いたり、コマンドの**入力**、ウェブの**検索**、nginx の設定の**編集**を頼んだりできます。",
              "\n\n---\n\n以上是示範裡預先寫好的修復過程。你還可以問我**磁碟**還剩多少空間、**螢幕**上有什麼，或者讓我**輸入**一條命令、**搜尋**網頁、**編輯** nginx 設定。",
            )
          : ""),
      calls: [],
      finish: "stop",
    };
  }

  /** The website's demo: the answer to a message without a keyword, which says what the demo answers. */
  function visitorOverview(): Plan {
    return {
      reasoning: say(
        "Say what this demo can answer.",
        "说明这个演示能回答什么。",
        "このデモで答えられることを伝える。",
        "說明這個示範能回答什麼。",
      ),
      text: say(
        "This is a demo with sample servers: nothing here connects anywhere, and my replies are scripted, so I answer only a few requests. Try one of these:\n\n- Users are getting 502s from nginx on this box. Find out why and fix it.\n- How much disk space is left?\n- What’s on my screen?\n- Type a command into the shell.\n- Search the web for nginx upstream errors.\n- Edit the nginx config.\n\nIn the app, the assistant answers with the model you set up.",
        "这是一个使用示例服务器的演示：它不会连接任何地方，我的回复也是预先写好的，所以只能回应几种请求。可以试试：\n\n- 这台机器上的 nginx 一直返回 502，找出原因并修好它。\n- 磁盘还剩多少空间？\n- 屏幕上显示了什么？\n- 在 shell 里输入一条命令。\n- 搜索 nginx upstream 的报错。\n- 编辑 nginx 配置。\n\n在应用里，助手会用你设置的模型来回答。",
        "サンプルのサーバーを使ったデモです。どこにも接続せず、返答もあらかじめ用意したものなので、答えられる依頼は限られています。次のどれかを試してください。\n\n- nginx が 502 を返しています。原因を調べて直してください。\n- ディスクの空きはどれくらいですか？\n- 画面に何が表示されていますか？\n- シェルにコマンドを入力してください。\n- nginx の upstream エラーを検索してください。\n- nginx の設定を編集してください。\n\nアプリでは、設定したモデルがアシスタントとして答えます。",
        "這是使用範例伺服器的示範：它不會連線到任何地方，我的回覆也是預先寫好的，所以只能回應幾種請求。可以試試：\n\n- 這台機器上的 nginx 一直回傳 502，找出原因並修好它。\n- 磁碟還剩多少空間？\n- 螢幕上顯示了什麼？\n- 在 shell 裡輸入一條命令。\n- 搜尋 nginx upstream 的錯誤。\n- 編輯 nginx 設定。\n\n在應用程式裡，助理會用你設定的模型來回答。",
      ),
      calls: [],
      finish: "stop",
    };
  }

  /** The website's demo: the answer from what each call of the turn returned. */
  function visitorAnswer(since: AiEntryView[], results: ToolEntry[]): Plan {
    const calls = new Map(since.flatMap((e) => (e.role === "assistant" ? e.tool_calls : [])).map((x) => [x.id, x]));
    const edited = results.some((r) => calls.get(r.tool_call_id)?.name === "edit_file");
    const parts = results.flatMap((r): string[] => {
      const x = calls.get(r.tool_call_id);
      if (!x) return [];
      if (r.status === "rejected")
        return [
          say(
            "You declined this step, so it didn’t run.",
            "你拒绝了这一步，所以它没有运行。",
            "この操作は拒否されたので、実行していません。",
            "你拒絕了這一步，所以它沒有執行。",
          ),
        ];
      if (r.status !== "ok")
        return [
          say("This step didn’t finish.", "这一步没有完成。", "この操作は完了しませんでした。", "這一步沒有完成。"),
        ];
      switch (x.name) {
        case "run_command": {
          if (x.arguments.includes(DISK_COMMAND.unix))
            return [
              say(
                "The root filesystem is **48% full**: 23 GB of 49 GB used, 24 GB free. Nothing needs cleaning up yet.\n\n| Filesystem | Size | Used | Free | Use |\n|---|---|---|---|---|\n| `/` on `/dev/nvme0n1p1` | 49G | 23G | 24G | 48% |",
                "根分区已用 **48%**：49 GB 里用了 23 GB，还剩 24 GB，暂时不用清理。\n\n| 文件系统 | 大小 | 已用 | 可用 | 使用率 |\n|---|---|---|---|---|\n| `/`（`/dev/nvme0n1p1`） | 49G | 23G | 24G | 48% |",
                "ルートファイルシステムの使用率は **48%** です。49 GB のうち 23 GB を使っていて、空きは 24 GB。まだ整理する必要はありません。\n\n| ファイルシステム | サイズ | 使用 | 空き | 使用率 |\n|---|---|---|---|---|\n| `/`（`/dev/nvme0n1p1`） | 49G | 23G | 24G | 48% |",
                "根分割區已用 **48%**：49 GB 裡用了 23 GB，還剩 24 GB，暫時不用清理。\n\n| 檔案系統 | 大小 | 已用 | 可用 | 使用率 |\n|---|---|---|---|---|\n| `/`（`/dev/nvme0n1p1`） | 49G | 23G | 24G | 48% |",
              ),
            ];
          const stdout = r.content.split("--- stdout ---\n")[1]?.split("\n--- stderr ---")[0] ?? "";
          return [
            `${say("Here’s the space on this computer’s drives:", "这台电脑各个磁盘的空间：", "このコンピューターのドライブの容量です。", "這台電腦各個磁碟的空間：")}\n\n\`\`\`\n${stdout}\n\`\`\``,
          ];
        }
        case "read_terminal": {
          const screen = r.content
            .split("\n\n")
            .slice(1)
            .join("\n\n")
            .split("\n")
            .filter((l) => l.trim())
            .slice(-8);
          const missing = screen.some((l) => l.includes("command not found"));
          return [
            `${say("Here’s the end of your screen:", "屏幕最后几行是：", "画面の最後の数行です。", "螢幕最後幾行是：")}\n\n\`\`\`\n${screen.join("\n")}\n\`\`\`\n\n${
              missing
                ? say(
                    "A command there wasn’t found: this demo’s shell knows only a few, and `help` lists them.",
                    "其中有条命令没有找到：这个演示里的 shell 只认识几条命令，输入 `help` 可以看到。",
                    "見つからなかったコマンドがあります。このデモのシェルが知っているコマンドは少しだけで、`help` で一覧できます。",
                    "其中有條命令沒有找到：這個示範裡的 shell 只認識幾條命令，輸入 `help` 可以看到。",
                  )
                : say(
                    "Nothing there looks like an error.",
                    "看起来没有报错。",
                    "エラーらしいものは見当たりません。",
                    "看起來沒有錯誤。",
                  )
            }`,
          ];
        }
        case "send_input":
          return [
            say(
              "I typed `uptime` into your shell. The server has been **up for 41 days**, and its load average is 0.21, so it’s mostly idle.",
              "我在你的 shell 里输入了 `uptime`。这台服务器已经**运行了 41 天**，平均负载 0.21，基本空闲。",
              "シェルに `uptime` と入力しました。サーバーは **41 日間稼働**していて、ロードアベレージは 0.21。ほぼ待機状態です。",
              "我在你的 shell 裡輸入了 `uptime`。這台伺服器已經**運作了 41 天**，平均負載 0.21，幾乎閒置。",
            ),
          ];
        case "web_search":
          return [
            say(
              "The top results agree that a 502 from nginx usually comes from the upstream, not from nginx:\n\n1. **nginx 502 Bad Gateway: upstream prematurely closed connection**: the upstream closed the connection before answering, so check its logs and timeouts.\n2. **Module ngx_http_upstream_module**: nginx’s reference for `keepalive`, `max_fails`, and `fail_timeout`.\n3. **Debugging 502 errors behind a reverse proxy**: check that the upstream listens, doesn’t crash, and answers within the proxy’s timeouts.",
              "搜索结果的看法一致：nginx 返回 502，问题通常出在上游，而不是 nginx 本身：\n\n1. **nginx 502 Bad Gateway: upstream prematurely closed connection**：上游在回应前关闭了连接，先查它的日志和超时设置。\n2. **Module ngx_http_upstream_module**：nginx 关于 `keepalive`、`max_fails` 和 `fail_timeout` 的文档。\n3. **Debugging 502 errors behind a reverse proxy**：确认上游在监听、没有崩溃，并且在代理的超时时间内回应。",
              "検索結果はどれも、nginx の 502 は nginx 自体ではなく上流が原因のことが多いとしています。\n\n1. **nginx 502 Bad Gateway: upstream prematurely closed connection**: 上流が応答する前に接続を閉じている。上流のログとタイムアウトを確認する。\n2. **Module ngx_http_upstream_module**: `keepalive`、`max_fails`、`fail_timeout` についての nginx のリファレンス。\n3. **Debugging 502 errors behind a reverse proxy**: 上流が待ち受けているか、落ちていないか、プロキシのタイムアウト内に応答しているかを確認する。",
              "搜尋結果的看法一致：nginx 回傳 502，問題通常出在上游，而不是 nginx 本身：\n\n1. **nginx 502 Bad Gateway: upstream prematurely closed connection**：上游在回應前關閉了連線，先查它的日誌和逾時設定。\n2. **Module ngx_http_upstream_module**：nginx 關於 `keepalive`、`max_fails` 和 `fail_timeout` 的文件。\n3. **Debugging 502 errors behind a reverse proxy**：確認上游在監聽、沒有崩潰，並且在代理的逾時時間內回應。",
            ),
          ];
        case "read_file":
          return edited
            ? []
            : [
                say(
                  "This is nginx’s default site. It listens on port 80, serves files from `/var/www/html`, and passes `/api/` to `127.0.0.1:8081` with a 30-second read timeout.",
                  "这是 nginx 的默认站点。它监听 80 端口，从 `/var/www/html` 提供文件，并把 `/api/` 转发到 `127.0.0.1:8081`，读取超时为 30 秒。",
                  "nginx のデフォルトサイトです。ポート 80 で待ち受け、`/var/www/html` のファイルを返し、`/api/` を読み取りタイムアウト 30 秒で `127.0.0.1:8081` に転送しています。",
                  "這是 nginx 的預設網站。它監聽 80 連接埠，從 `/var/www/html` 提供檔案，並把 `/api/` 轉送到 `127.0.0.1:8081`，讀取逾時為 30 秒。",
                ),
              ];
        case "edit_file":
          return [
            say(
              "I changed `/etc/nginx/sites-available/default`: `/api/` now goes to port **8080**, the read timeout is 60 seconds, and nginx passes the `Host` header on. The diff is above, where you can revert it. Reload nginx to apply it:\n\n```bash\nsudo nginx -t && sudo systemctl reload nginx\n```",
              "我改了 `/etc/nginx/sites-available/default`：`/api/` 现在转发到 **8080** 端口，读取超时改为 60 秒，并把 `Host` 头传给上游。改动的 diff 在上面，可以在那里撤销。重新加载 nginx 后生效：\n\n```bash\nsudo nginx -t && sudo systemctl reload nginx\n```",
              "`/etc/nginx/sites-available/default` を変更しました。`/api/` の転送先はポート **8080** になり、読み取りタイムアウトは 60 秒、`Host` ヘッダーも上流に渡します。差分は上にあり、そこから元に戻せます。反映するには nginx をリロードします。\n\n```bash\nsudo nginx -t && sudo systemctl reload nginx\n```",
              "我改了 `/etc/nginx/sites-available/default`：`/api/` 現在轉送到 **8080** 連接埠，讀取逾時改為 60 秒，並把 `Host` 標頭傳給上游。變更的 diff 在上面，可以在那裡復原。重新載入 nginx 後生效：\n\n```bash\nsudo nginx -t && sudo systemctl reload nginx\n```",
            ),
          ];
        default:
          return [];
      }
    });
    if (parts.length === 0) return visitorOverview();
    return {
      reasoning: say(
        "Answer from what the tools returned.",
        "根据工具返回的结果回答。",
        "ツールの結果から答える。",
        "根據工具傳回的結果回答。",
      ),
      text: parts.join("\n\n"),
      calls: [],
      finish: "stop",
    };
  }

  async function plan(c: Conv, turn: Turn): Promise<Plan> {
    const user = lastUser(c);
    const since = c.entries.slice(user.index + 1);
    const results = since.filter((e): e is ToolEntry => e.role === "tool");
    const callsSoFar = since.reduce((n, e) => n + (e.role === "assistant" ? e.tool_calls.length : 0), 0);
    // AI-10: what is attached to the message is in blocks before the typed text; keywords come from the typed text.
    const { attachments, typed } = parseMessage(user.text);
    const diagnostics = attachments.find((a) => a.kind === "diagnostics");
    const attachment = attachments.find((a) => a.kind === "selection");
    const added = attachments.filter((a) => a.kind === "paste" || a.kind === "file");
    const errorLine = diagnostics?.text.split("\n").find((line) => line.startsWith("Error:")) ?? "";
    const words = typed.toLowerCase();
    const want = (...w: string[]) => w.some((x) => words.includes(x));
    /** A keyword for reviewing the app, which the website's demo leaves to the overview it answers with. */
    const review = (...w: string[]) => !VISITOR && want(...w);
    const terminal = turn.context.tab;

    if (flags.has("refused"))
      return {
        reasoning: "",
        text: sayZh(
          "I can’t help with that request.",
          "抱歉，我不能帮助完成这个请求。",
          "抱歉，我無法協助完成這個請求。",
        ),
        calls: [],
        finish: "refused",
      };
    if (flags.has("length")) {
      const para = sayZh(
        "This host runs several services; here is each one with its configuration, log location and the usual ways to debug it. ",
        "这台主机上运行着多个服务，下面逐一说明它们的配置、日志位置和常见问题的排查方法。",
        "這台主機上執行著多個服務，下面逐一說明它們的設定、日誌位置和常見問題的排解方法。",
      );
      return {
        reasoning: sayZh("This needs a long write-up.", "需要写一份很长的说明。", "需要寫一份很長的說明。"),
        text: `## ${sayZh("Services", "服务总览", "服務總覽")}\n\n${para.repeat(14)}`,
        calls: [],
        finish: "length",
      };
    }
    // The website's demo starts the fix from the question its input suggests.
    if ((flags.has("showcase") || (VISITOR && want("502"))) && terminal) {
      // The script goes on only while each step runs; a declined one ends it.
      const stopped = results.find((r) => r.status !== "ok");
      if (stopped)
        return {
          reasoning: "",
          text:
            stopped.status === "rejected"
              ? say(
                  "You declined that step, so I stopped there. Send the question again to start over.",
                  "你拒绝了这一步，所以我停在这里。重新发送问题就能从头开始。",
                  "この操作は拒否されたので、ここで止めました。もう一度質問を送ると最初からやり直します。",
                  "你拒絕了這一步，所以我停在這裡。重新傳送問題就能從頭開始。",
                )
              : say(
                  "That step didn’t finish, so I stopped there. Send the question again to start over.",
                  "这一步没有完成，所以我停在这里。重新发送问题就能从头开始。",
                  "この操作が完了しなかったので、ここで止めました。もう一度質問を送ると最初からやり直します。",
                  "這一步沒有完成，所以我停在這裡。重新傳送問題就能從頭開始。",
                ),
          calls: [],
          finish: "stop",
        };
      return showcase(since.filter((e) => e.role === "assistant").length);
    }

    if (results.length === 0 && callsSoFar === 0) {
      const calls: AiToolCall[] = [];
      // Terminal tools only with a connected tab; a request without one does not offer them.
      const wantsTerminal = want(
        ...[
          "disk",
          "space",
          "磁盘",
          "空间",
          "磁碟",
          "空間",
          "ディスク",
          "容量",
          "df",
          "fail",
          "失败",
          "失敗",
          "sleep",
          "workdir",
          "工作目录",
          "工作目錄",
          "作業フォルダ",
          "screen",
          "屏幕",
          "螢幕",
          "画面",
          "type",
          "send",
          "输入",
          "輸入",
          "入力",
        ],
        ...["read file", "读取文件", "讀取檔案", "ファイルを読", "edit", "编辑", "編輯", "編集"],
        ...["write", "写入", "寫入", "書き込", "grep", "glob"],
      );
      if (terminal) {
        if (want("disk", "space", "磁盘", "空间", "磁碟", "空間", "ディスク", "容量", "df"))
          calls.push(
            call("run_command", { command: DISK_COMMAND[turn.context.local?.kind ?? "unix"], timeout_seconds: 30 }),
          );
        if (review("fail", "失败", "失敗"))
          calls.push(call("run_command", { command: "sudo systemctl restart nonexistent.service" }));
        if (review("sleep")) calls.push(call("run_command", { command: "sleep 8 && echo done", timeout_seconds: 30 }));
        if (review("workdir", "工作目录", "工作目錄", "作業フォルダ"))
          calls.push(call("run_command", { command: "npm test", workdir: "/srv/app", timeout_seconds: 120 }));
        if (want("screen", "屏幕", "螢幕", "画面")) calls.push(call("read_terminal", { lines: 50 }));
        if (want("type", "send", "输入", "輸入", "入力"))
          calls.push(call("send_input", { text: "uptime", keys: ["enter"], wait_seconds: 5 }));
        if (review("hidden", "隐藏", "隱藏", "隠し"))
          calls.push(
            call("run_command", {
              command: "cd /srv/app\tmake deploy\nprintf '\x1b[2J'\necho ‮/ fr- mr\n",
              timeout_seconds: 30,
            }),
          );
        // AI-38…40: the file tools, on the mock's files. A change to an existing file reads it first, as
        // the model must; "unread" skips that to show the refusal.
        const NGINX = "/etc/nginx/sites-available/default";
        const readFirst = (path: string) => {
          if (!calls.some((c) => c.name === "read_file" && c.arguments.includes(JSON.stringify(path))))
            calls.push(call("read_file", { path }));
        };
        if (want("read file", "读取文件", "讀取檔案", "ファイルを読") || review("cat ")) readFirst(NGINX);
        if (review("glob")) calls.push(call("glob", { pattern: "sites-*/*", path: "/etc/nginx" }));
        if (review("grep"))
          calls.push(call("grep", { pattern: "proxy_\\w+", path: "/etc", output_mode: "content", context: 1 }));
        if (review("unread"))
          calls.push(call("edit_file", { path: "/etc/motd", old_string: "Sundays", new_string: "Saturdays" }));
        if (want("edit", "编辑", "編輯", "編集")) readFirst(NGINX);
        if (want("edit", "编辑", "編輯", "編集"))
          calls.push(
            call("edit_file", {
              path: "/etc/nginx/sites-available/default",
              old_string: "        proxy_pass http://127.0.0.1:8081/;\n        proxy_read_timeout 30s;",
              new_string:
                "        proxy_pass http://127.0.0.1:8080/;\n        proxy_read_timeout 60s;\n        proxy_set_header Host $host;",
            }),
          );
        if (review("listen", "端口", "連接埠", "ポート", "nomatch", "不匹配", "不符", "不一致")) readFirst(NGINX);
        if (review("crlf")) readFirst("/srv/app/web.config");
        if (review("motd")) readFirst("/etc/motd");
        if (review("listen", "端口", "連接埠", "ポート"))
          calls.push(
            call("edit_file", {
              path: "/etc/nginx/sites-available/default",
              old_string: "80 default_server",
              new_string: "8080 default_server",
              replace_all: true,
            }),
          );
        if (review("nomatch", "不匹配", "不符", "不一致"))
          calls.push(
            call("edit_file", {
              path: "/etc/nginx/sites-available/default",
              old_string: "listen 443 ssl;",
              new_string: "listen 8443 ssl;",
            }),
          );
        if (review("crlf"))
          calls.push(
            call("edit_file", {
              path: "/srv/app/web.config",
              old_string: '<add key="mode" value="production" />',
              new_string: '<add key="mode" value="maintenance" />\n    <add key="banner" value="Back soon" />',
            }),
          );
        if (review("write", "写入", "寫入", "書き込"))
          calls.push(
            call("write_file", {
              path: MAINTENANCE,
              content:
                "<!doctype html>\n<title>Maintenance</title>\n<h1>We’ll be back soon</h1>\n<p>Scheduled maintenance until 04:00 UTC.</p>\n",
            }),
          );
        if (review("bigfile"))
          calls.push(
            call("write_file", {
              path: "/srv/app/data/seed.csv",
              content: `id,name\n${Array.from({ length: 1200 }, (_, i) => `${i + 1},host-${i + 1}`).join("\n")}\n`,
            }),
          );
        if (review("motd"))
          calls.push(
            call("write_file", {
              path: "/etc/motd",
              content: "Welcome to prod-api.\nMaintenance window: Saturdays 01:00–03:00 UTC.\nOn call: #ops\n",
            }),
          );
        if (review("cutshort"))
          calls.push(
            call("write_file", {
              path: CUT_SHORT,
              content: "# Cache settings\nmax_size = 512m\ninactive = 60m\nuse_temp_path = off\n",
            }),
          );
        if (review("rmfile")) calls.push(call("run_command", { command: "rm /etc/motd" }));
        if (review("symlink"))
          calls.push(call("run_command", { command: `ln -sfn /srv/app/shared/maintenance.html ${MAINTENANCE}` }));
      }
      if (want("search", "搜索", "搜尋", "検索"))
        calls.push(call("web_search", { query: "nginx 502 bad gateway upstream prematurely closed" }));
      if (review("fetch", "url", "网页", "網頁", "ページ"))
        calls.push(
          call("fetch_url", {
            url: "https://nginx.org/en/docs/http/ngx_http_upstream_module.html",
            ...(want("whole", "整页", "整頁", "全体") ? { full_page: true } : {}),
          }),
        );
      if (review("mcp")) {
        const mcp = await mcpCall(turn, words);
        // AI-45: "deferred" searches the MCP tools, then calls one through `call_mcp_tool`.
        if (want("deferred")) {
          calls.push(call("search_tools", { query: "issues" }));
          calls.push(call("call_mcp_tool", { tool: mcp.name, arguments: JSON.parse(mcp.arguments) as unknown }));
        } else calls.push(mcp);
      }
      if (review("unknown")) calls.push(call("delete_everything", { confirm: true }));
      // Connection diagnostics: read the built-in skill's troubleshooting reference first.
      if (diagnostics && calls.length === 0)
        calls.push(call("read_skill", { name: "hatoba", path: "references/troubleshooting-connections.md" }));
      if (calls.length > 0)
        return {
          reasoning: say(
            "The user wants to know about the host. Gather the facts with tools first, then answer from the results.",
            "用户想了解主机的状态。先用工具收集信息，再根据结果回答。",
            "ユーザーはホストの状態を知りたがっている。まずツールで情報を集め、その結果から答える。",
            "使用者想了解主機的狀態。先用工具收集資訊，再根據結果回答。",
          ),
          text: say("Let me check.", "我先看一下。", "確認します。", "我先看一下。"),
          calls,
          finish: "tool_calls",
        };
      if (wantsTerminal)
        return {
          reasoning: "",
          text: say(
            "No terminal tab is attached to this conversation, so I can’t run commands or read the screen. Open a host and ask me again.",
            "这个对话没有连接终端标签页，所以我无法运行命令或读取屏幕。请打开一台主机，然后再问我一次。",
            "この会話にはターミナルのタブがつながっていないため、コマンドの実行も画面の読み取りもできません。ホストを開いてから、もう一度聞いてください。",
            "這個對話沒有連線終端機分頁，所以我無法執行命令或讀取螢幕。請開啟一台主機，然後再問我一次。",
          ),
          calls: [],
          finish: "stop",
        };
    }

    if (results.length > 0 && diagnostics) {
      return {
        reasoning: say(
          "Use the diagnostics and the troubleshooting reference to explain the cause and the next steps.",
          "根据诊断信息和故障排查参考给出原因和下一步。",
          "診断情報とトラブルシューティングの資料から、原因と次の手順を説明する。",
          "根據診斷資訊和疑難排解資料說明原因和下一步。",
        ),
        text: say(
          `The diagnostics say \`${errorLine}\`.\n\nLikely causes:\n\n1. The host is down, or its SSH server is not running\n2. A firewall or security group blocks the port\n3. The address or port is wrong\n\nCheck that the host answers at all, then the address and port in **Edit Host**, and choose **Retry** after fixing them.`,
          `诊断信息显示：\`${errorLine}\`。\n\n可能的原因：\n\n1. 主机没有开机，或者 SSH 服务没有运行\n2. 防火墙或安全组挡住了端口\n3. 地址或端口填错了\n\n先确认能 ping 通主机，再检查 **编辑主机** 里的地址和端口。改好之后点 **重试**。`,
          `診断情報には \`${errorLine}\` とあります。\n\n考えられる原因:\n\n1. ホストが起動していないか、SSH サーバーが動いていない\n2. ファイアウォールやセキュリティグループがポートを塞いでいる\n3. アドレスかポートが間違っている\n\nまずホストが応答するか確かめてから、**ホストを編集** でアドレスとポートを確認してください。直したら **再試行** を選びます。`,
          `診斷資訊顯示：\`${errorLine}\`。\n\n可能的原因：\n\n1. 主機沒有開機，或者 SSH 服務沒有執行\n2. 防火牆或安全群組擋住了連接埠\n3. 位址或連接埠填錯了\n\n先確認能 ping 通主機，再檢查 **編輯主機** 裡的位址和連接埠。改好之後按 **重試**。`,
        ),
        calls: [],
        finish: "stop",
      };
    }

    if (results.length > 0 && VISITOR) return visitorAnswer(since, results);
    if (results.length > 0) {
      const parts = results.map((r) => {
        const head = r.content.split("\n").slice(0, 8).join("\n");
        const label = r.status === "ok" ? "" : ` _(${r.status})_`;
        return `- \`${r.tool_call_id}\`${label}\n\n  \`\`\`\n${head.replace(/^/gm, "  ")}\n  \`\`\``;
      });
      return {
        reasoning: sayZh("Summarize what the tools returned.", "根据工具结果整理答案。", "根據工具結果整理答案。"),
        text: sayZh(
          `Here is what I found:\n\n${parts.join("\n")}\n\n| Item | State |\n|---|---|\n| Root filesystem | 48% used, fine |\n| Load | 0.21, idle |\n\nTell me if you want me to do anything else.`,
          `这是我看到的结果：\n\n${parts.join("\n")}\n\n| 项目 | 状态 |\n|---|---|\n| 根分区 | 已用 48%，正常 |\n| 负载 | 0.21，空闲 |\n\n如果还需要我做什么，告诉我。`,
          `這是我看到的結果：\n\n${parts.join("\n")}\n\n| 項目 | 狀態 |\n|---|---|\n| 根分割區 | 已用 48%，正常 |\n| 負載 | 0.21，閒置 |\n\n如果還需要我做什麼，告訴我。`,
        ),
        calls: [],
        finish: "stop",
      };
    }

    if (added.length > 0) {
      const list = added
        .map(
          (a) =>
            `- ${a.kind === "file" ? `\`${a.name}\`` : say("pasted text", "粘贴的文本", "貼り付けたテキスト", "貼上的文字")}: ${a.lines} ${say("line(s)", "行", "行", "行")}, ${[...a.text].length} ${say("characters", "个字符", "文字", "個字元")}`,
        )
        .join("\n");
      return {
        reasoning: say(
          "The user attached text; answer from it.",
          "用户附上了文本，根据它回答。",
          "ユーザーがテキストを添付した。それをもとに答える。",
          "使用者附上了文字，根據它回答。",
        ),
        text: say(
          `I received these attachments:\n\n${list}\n\nAll of it arrived whole. Tell me what to look for in it.`,
          `我收到了这些附件：\n\n${list}\n\n内容都完整地到了。告诉我要从里面找什么。`,
          `次の添付を受け取りました。\n\n${list}\n\nすべて欠けずに届いています。何を探せばよいか教えてください。`,
          `我收到了這些附件：\n\n${list}\n\n內容都完整地到了。告訴我要從裡面找什麼。`,
        ),
        calls: [],
        finish: "stop",
      };
    }

    if (attachment) {
      const first = "```\n" + attachment.text.split("\n")[0] + "\n```";
      const cut = attachment.truncated
        ? say(
            " (long, so the middle was left out)",
            "（太长，中间部分被省略了）",
            "（長いので途中を省略しました）",
            "（太長，中間部分被省略了）",
          )
        : "";
      return {
        reasoning: say(
          "The user attached text selected in the terminal; answer from it.",
          "用户附上了终端里选中的内容，根据它回答。",
          "ユーザーがターミナルで選択したテキストを添付した。それをもとに答える。",
          "使用者附上了終端機裡選取的內容，根據它回答。",
        ),
        text: say(
          `You selected ${attachment.lines} line(s) on **${attachment.host}**${cut}. The first one is:\n\n${first}\n\nIt looks like ordinary terminal output. Tell me what to look into next.`,
          `你从 **${attachment.host}** 选中了 ${attachment.lines} 行${cut}。第一行是：\n\n${first}\n\n这看起来是一段正常的终端输出。如果要我进一步检查，告诉我要看什么。`,
          `**${attachment.host}** で ${attachment.lines} 行を選択しました${cut}。最初の行は次のとおりです。\n\n${first}\n\nふつうのターミナル出力のようです。次に何を調べるか教えてください。`,
          `你從 **${attachment.host}** 選取了 ${attachment.lines} 行${cut}。第一行是：\n\n${first}\n\n這看起來是一段正常的終端機輸出。如果要我進一步檢查，告訴我要看什麼。`,
        ),
        calls: [],
        finish: "stop",
      };
    }

    if (VISITOR) return visitorOverview();
    return {
      reasoning: sayZh(
        "A general question; no tools needed. Answer briefly in Markdown.",
        "这是一般性的问题，不需要工具。用 Markdown 简短回答。",
        "這是一般性的問題，不需要工具。用 Markdown 簡短回答。",
      ),
      text: sayZh(
        "I’m Hatoba’s AI assistant (demo mode). I can:\n\n1. **Read the screen** and explain errors\n2. Run **commands** such as `df -h`\n3. **Type** into the shell\n4. **Search** and read the web\n\n```bash\n# for example\njournalctl -u nginx -n 50 --no-pager\n```\n\nTry asking about **disk** usage, what’s on the **screen**, to **search** for an error, or to **type** a command. See the [Hatoba repository](https://github.com/scarletkc/Hatoba). Remote images are never loaded: ![diagram](https://example.com/diagram.png)\n\n> Raw HTML shows as text: <b>not bold</b>",
        "我是 Hatoba 的 AI 助手（演示模式）。我可以：\n\n1. **读取屏幕**，解释报错\n2. 运行 `df -h` 之类的**命令**\n3. 在 shell 里**输入**内容\n4. **搜索**和读取网页\n\n```bash\n# 例如\njournalctl -u nginx -n 50 --no-pager\n```\n\n试试问我**磁盘**用量、**屏幕**上的内容、**搜索**某个错误，或者让我**输入**一条命令。文档见 [Hatoba 仓库](https://github.com/scarletkc/Hatoba)。远程图片不会加载：![架构图](https://example.com/diagram.png)\n\n> 原始 HTML 只会显示为文字：<b>不加粗</b>",
        "我是 Hatoba 的 AI 助理（示範模式）。我可以：\n\n1. **讀取螢幕**，解釋錯誤\n2. 執行 `df -h` 之類的**命令**\n3. 在 shell 裡**輸入**內容\n4. **搜尋**和讀取網頁\n\n```bash\n# 例如\njournalctl -u nginx -n 50 --no-pager\n```\n\n試試問我**磁碟**用量、**螢幕**上的內容、**搜尋**某個錯誤，或者讓我**輸入**一條命令。文件見 [Hatoba 儲存庫](https://github.com/scarletkc/Hatoba)。遠端圖片不會載入：![架構圖](https://example.com/diagram.png)\n\n> 原始 HTML 只會顯示為文字：<b>不會變粗體</b>",
      ),
      calls: [],
      finish: "stop",
    };
  }

  return { sideAnswer, plan };
}
