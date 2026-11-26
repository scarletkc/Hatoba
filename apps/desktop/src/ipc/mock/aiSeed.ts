import type { AiAutoVerdict, AiEntryView, AiToolStatus } from "../types";
import { DAY, HOUR, MIN, type AiMockCtx, type Assistant, type Conv } from "./aiShared";

export function createAiSeed(ctx: AiMockCtx) {
  const { flags, sayZh, convs, replyTerminal, newId, call } = ctx;

  function seed() {
    const now = Date.now();
    const add = (
      id: string,
      title: string,
      host: string | null,
      ago: number,
      pinned: boolean,
      build: (at: (offset: number) => number) => AiEntryView[],
    ) => {
      const created = now - ago;
      const at = (offset: number) => created + offset;
      const c: Conv = {
        // The pinned one was last sent at High (AI-05).
        view: {
          id,
          title,
          host_id: host,
          quick_target: null,
          origin: null,
          local: null,
          pinned,
          context_start: null,
          effort: pinned ? "high" : null,
          created_at: created,
          updated_at: created,
        },
        entries: build(at),
        runningUntil: 0,
      };
      convs.set(id, c);
      return c;
    };
    const user = (ms: number, text: string): AiEntryView => ({
      role: "user",
      entry_id: newId(ms),
      created_at: ms,
      text,
    });
    const reply = (ms: number, model: [string, string], text: string, opts: Partial<Assistant> = {}): Assistant => {
      const entry: Assistant = {
        role: "assistant",
        entry_id: newId(ms),
        created_at: ms,
        provider_id: model[0],
        model_id: model[1],
        text,
        reasoning: null,
        tool_calls: [],
        finish: "stop",
        usage: { input_tokens: 3200, output_tokens: 240, estimated: false },
        ...opts,
      };
      // Every demo conversation was held in a connected tab of its host.
      replyTerminal.set(entry.entry_id, true);
      return entry;
    };
    const result = (
      ms: number,
      callId: string,
      status: AiToolStatus,
      content: string,
      auto: AiAutoVerdict | null = null,
    ): AiEntryView => ({
      role: "tool",
      entry_id: newId(ms),
      created_at: ms,
      tool_call_id: callId,
      status,
      content,
      auto,
    });
    // AI-47: verdicts of the judge of Auto mode, as Rust stores them with the results of calls Auto decided.
    const judged = (
      decision: AiAutoVerdict["decision"],
      answers: [string, number][],
      concerns: string[] = [],
    ): AiAutoVerdict => ({
      provider_id: "p-anthropic",
      model_id: "claude-haiku-5-5",
      decision,
      answers: answers.map(([question, probability]) => ({ question, probability })),
      concerns,
      failure: null,
    });
    const SONNET: [string, string] = ["p-anthropic", "claude-sonnet-5-5"];
    const QWEN: [string, string] = ["p-ollama", "qwen3:8b"];
    const OSS: [string, string] = ["p-ollama", "gpt-oss:20b"];

    add(
      "c-nginx",
      sayZh("Debug nginx 502 on prod-api", "排查 prod-api 上的 nginx 502", "排查 prod-api 上的 nginx 502"),
      "h-api-tokyo",
      2 * DAY,
      true,
      (at) => {
        const logs = call("run_command", { command: "sudo journalctl -u nginx -n 20 --no-pager", timeout_seconds: 30 });
        const screen = call("read_terminal", { lines: 100 });
        return [
          user(
            at(0),
            sayZh(
              "The site keeps returning 502. Can you look at nginx?",
              "网站一直返回 502，帮我看看 nginx",
              "網站一直回傳 502，幫我看看 nginx",
            ),
          ),
          reply(
            at(MIN),
            SONNET,
            sayZh(
              "I’ll check the nginx log and the current screen first.",
              "我先看 nginx 的日志和当前屏幕。",
              "我先看 nginx 的日誌和目前的螢幕。",
            ),
            {
              reasoning: sayZh(
                "A 502 usually means the upstream failed. Start with nginx’s error log.",
                "502 通常是上游服务的问题。先看 nginx 错误日志。",
                "502 通常是上游服務的問題。先看 nginx 錯誤日誌。",
              ),
              tool_calls: [logs, screen],
              finish: "tool_calls",
            },
          ),
          result(
            at(2 * MIN),
            logs.id,
            "ok",
            'exit status: 0\n--- stdout ---\nOct 06 09:12:01 prod-api-tokyo nginx[812]: upstream prematurely closed connection while reading response header from upstream, client: 10.0.0.4, upstream: "http://127.0.0.1:8080/v1/hosts"\n--- stderr ---\n',
            // Manual approval asks about `sudo`; Auto's judge found that it only reads.
            judged("run", [
              ["requested", 0.93],
              ["forbidden", 0.02],
              ["changes_anything", 0.04],
              ["destroys_data", 0.01],
              ["touches_secrets", 0.06],
              ["sends_data_out", 0.01],
              ["security_change", 0.02],
              ["runs_unseen_code", 0.01],
            ]),
          ),
          result(
            at(2 * MIN + 1000),
            screen.id,
            "ok",
            "Alternate screen: no. This is all of the terminal's 1 line(s).\n\ndeploy@prod-api-tokyo:/srv/api$",
          ),
          reply(
            at(3 * MIN),
            SONNET,
            sayZh(
              "The upstream `api.service` (127.0.0.1:8080) closed the connection before sending headers.\n\n| Check | Result |\n|---|---|\n| nginx | running |\n| upstream | **closed early** |\n\nNext, look at `journalctl -u api -n 100` to see whether the service crashed and restarted.",
              "上游 `api.service`（127.0.0.1:8080）在返回响应头之前关闭了连接。\n\n| 检查项 | 结果 |\n|---|---|\n| nginx | 正常运行 |\n| 上游 | **提前关闭连接** |\n\n建议先看 `journalctl -u api -n 100`，确认服务是否崩溃重启。",
              "上游 `api.service`（127.0.0.1:8080）在傳回回應標頭之前關閉了連線。\n\n| 檢查項 | 結果 |\n|---|---|\n| nginx | 正常運作 |\n| 上游 | **提前關閉連線** |\n\n建議先看 `journalctl -u api -n 100`，確認服務是否當掉並重新啟動。",
            ),
            {
              reasoning: sayZh("The log points at the upstream.", "日志说明问题在上游。", "日誌說明問題在上游。"),
              usage: { input_tokens: 5400, output_tokens: 310, estimated: false },
            },
          ),
        ];
      },
    );

    const vacuum = add(
      "c-vacuum",
      sayZh("Postgres vacuum plan", "Postgres VACUUM 计划", "Postgres VACUUM 計畫"),
      "h-db-osaka-01",
      DAY + 3 * HOUR,
      false,
      (at) => [
        user(
          at(0),
          sayZh(
            "Help me plan VACUUM on db-osaka-01",
            "帮我规划一下 db-osaka-01 的 VACUUM",
            "幫我規劃一下 db-osaka-01 的 VACUUM",
          ),
        ),
        reply(
          at(MIN),
          QWEN,
          sayZh(
            "First find the most bloated tables… (a long analysis)",
            "先确认哪些表膨胀最严重……（很长的分析）",
            "先確認哪些表膨脹最嚴重……（很長的分析）",
          ),
          { usage: { input_tokens: 21_000, output_tokens: 3_100, estimated: false } },
        ),
        user(
          at(4 * MIN),
          sayZh("Go on, with advice for each table", "继续，给出每张表的建议", "繼續，給出每張表的建議"),
        ),
        reply(
          at(6 * MIN),
          QWEN,
          sayZh("Per-table advice… (long)", "按表给出的建议……（很长）", "按表給出的建議……（很長）"),
          {
            usage: { input_tokens: 29_500, output_tokens: 4_200, estimated: false },
          },
        ),
      ],
    );
    const summary: AiEntryView = {
      role: "summary",
      entry_id: newId(vacuum.view.created_at + 8 * MIN),
      created_at: vacuum.view.created_at + 8 * MIN,
      text: sayZh(
        "**Summary of the earlier conversation**: the user is planning VACUUM on db-osaka-01. `events` and `sessions` are the most bloated tables; run `VACUUM (ANALYZE)` on them off-peak and lower `autovacuum_vacuum_scale_factor` to 0.05.",
        "**之前的对话摘要**：用户要为 db-osaka-01 规划 VACUUM。已确认 `events` 和 `sessions` 两张表膨胀最严重，建议在凌晨低峰期对它们运行 `VACUUM (ANALYZE)`，并把 `autovacuum_vacuum_scale_factor` 调到 0.05。",
        "**之前的對話摘要**：使用者要為 db-osaka-01 規劃 VACUUM。已確認 `events` 和 `sessions` 兩張表膨脹最嚴重，建議在凌晨離峰時段對它們執行 `VACUUM (ANALYZE)`，並把 `autovacuum_vacuum_scale_factor` 調到 0.05。",
      ),
    };
    vacuum.entries.push(summary);
    vacuum.view.context_start = summary.entry_id;
    const vt = vacuum.view.created_at + 10 * MIN;
    vacuum.entries.push(
      user(
        vt,
        sayZh(
          "Can I run it on sessions right now?",
          "那 sessions 表现在能直接跑吗？",
          "那 sessions 表現在能直接跑嗎？",
        ),
      ),
      reply(
        vt + MIN,
        QWEN,
        sayZh(
          "You can, but it’s peak time now; wait until after 02:00.",
          "可以，但现在是业务高峰，建议等到 02:00 以后。",
          "可以，但現在是業務尖峰，建議等到 02:00 以後。",
        ),
        {
          usage: { input_tokens: 32_800, output_tokens: 160, estimated: false },
        },
      ),
    );

    add(
      "c-cleanup",
      sayZh("Clean up disk on staging", "清理 staging 磁盘空间", "清理 staging 磁碟空間"),
      "h-staging-worker",
      3 * HOUR,
      false,
      (at) => {
        const rm = call("run_command", { command: "sudo rm -rf /var/log/journal/*", timeout_seconds: 60 });
        const vacuumLogs = call("run_command", { command: "sudo journalctl --vacuum-size=200M" });
        const type = call("send_input", { text: "docker system prune -af", keys: ["enter"], wait_seconds: 30 });
        const fetch = call("fetch_url", { url: "https://docs.docker.com/engine/manage-resources/pruning/" });
        const mcp = call("mcp__github__create_issue", { repo: "scarletkc/infra", title: "staging disk full" });
        return [
          user(
            at(0),
            sayZh(
              "staging-worker-01 is nearly out of disk",
              "staging-worker-01 的磁盘快满了",
              "staging-worker-01 的磁碟快滿了",
            ),
          ),
          reply(
            at(MIN),
            OSS,
            sayZh(
              "I’ll clean up logs and the Docker cache.",
              "我来清理日志和 Docker 缓存。",
              "我來清理日誌和 Docker 快取。",
            ),
            {
              tool_calls: [rm, vacuumLogs, type, fetch, mcp],
              finish: "tool_calls",
              usage: null,
            },
          ),
          result(at(2 * MIN), rm.id, "rejected", "don't delete the journal by hand"),
          result(
            at(3 * MIN),
            vacuumLogs.id,
            "ok",
            'The user edited the arguments before the call ran; it ran with {"command":"sudo journalctl --vacuum-size=500M"}\n\nexit status: 0\n--- stdout ---\nVacuuming done, freed 1.2G of archived journals from /var/log/journal.\n--- stderr ---\n',
            // The card showed what concerned Auto's judge, and the user edited the call and ran it.
            judged(
              "ask",
              [
                ["requested", 0.4],
                ["forbidden", 0.03],
                ["changes_anything", 0.22],
                ["destroys_data", 0.64],
                ["touches_secrets", 0.02],
                ["sends_data_out", 0.01],
                ["security_change", 0.02],
                ["runs_unseen_code", 0.01],
              ],
              ["destroys_data"],
            ),
          ),
          result(
            at(4 * MIN),
            type.id,
            "error",
            "The terminal tab disconnected while waiting. Output before that:\n\ndocker system prune -af\nDeleted Containers:",
          ),
          result(at(5 * MIN), fetch.id, "cancelled", "The call was cancelled before it ran."),
          result(at(5 * MIN), mcp.id, "error", "No running MCP server offers the tool mcp__github__create_issue."),
          reply(
            at(6 * MIN),
            OSS,
            sayZh(
              "The journal is down to 500M, which freed 1.2G. The Docker prune stopped when the terminal disconnected; try again after reconnecting.",
              "日志已压缩到 500M，释放了 1.2G。Docker 清理在终端断开时中断了，重新连接后可以再试。",
              "日誌已壓縮到 500M，釋放了 1.2G。Docker 清理在終端機中斷連線時停止了，重新連線後可以再試。",
            ),
            {
              usage: { input_tokens: 2100, output_tokens: 90, estimated: true },
            },
          ),
        ];
      },
    );

    // On a quick connection (HOST-12), which the conversation records by its target (AI-09).
    const pi = add(
      "c-pi",
      sayZh("Raspberry Pi temperature", "树莓派的温度", "樹莓派的溫度"),
      null,
      5 * HOUR,
      false,
      (at) => [
        user(at(0), sayZh("How hot is this Pi right now?", "这台树莓派现在多热？", "這台樹莓派現在多熱？")),
        reply(
          at(MIN),
          OSS,
          sayZh(
            "`vcgencmd measure_temp` reads 52.1°C, which is within the normal range.",
            "`vcgencmd measure_temp` 显示 52.1°C，在正常范围内。",
            "`vcgencmd measure_temp` 顯示 52.1°C，在正常範圍內。",
          ),
          {
            usage: { input_tokens: 1800, output_tokens: 60, estimated: false },
          },
        ),
      ],
    );
    pi.view.quick_target = { username: "pi", address: "raspberrypi.local", port: 22 };

    const deploy = add(
      "c-deploy",
      sayZh("Watch the prod-api deploy", "盯一下 prod-api 的部署", "盯一下 prod-api 的部署"),
      "h-api-tokyo",
      20 * MIN,
      false,
      (at) => [
        user(
          at(0),
          sayZh("Tell me how the deploy went when it finishes", "部署跑完之后告诉我结果", "部署跑完之後告訴我結果"),
        ),
      ],
    );
    if (flags.has("running")) deploy.runningUntil = now + 20_000;
  }

  /** `?ai=running`: the turn an earlier page started finishes. */
  function settleRemote(c: Conv) {
    if (!c.runningUntil || Date.now() < c.runningUntil) return;
    c.runningUntil = 0;
    const entry: Assistant = {
      role: "assistant",
      entry_id: newId(),
      created_at: Date.now(),
      provider_id: "p-anthropic",
      model_id: "claude-sonnet-5-5",
      text: sayZh(
        "The deploy finished: all 3 instances are healthy and `/healthz` returns 200.",
        "部署完成：3 个实例都已健康，`/healthz` 返回 200。",
        "部署完成：3 個實例都已正常，`/healthz` 回傳 200。",
      ),
      reasoning: null,
      tool_calls: [],
      finish: "stop",
      usage: { input_tokens: 4100, output_tokens: 40, estimated: false },
    };
    // The earlier page ran the turn in a connected tab.
    replyTerminal.set(entry.entry_id, true);
    c.entries.push(entry);
  }

  return { seed, settleRemote };
}
