import { messages, type Locale, type MessageKey } from "./locales";

/**
 * The built-in `hatoba` skill (crates/hatoba-ai/src/skills/builtin/hatoba) names UI labels in English. Its
 * `references/labels/<locale>-<area>.md` files give them in every other language, so the assistant can quote
 * the label the user sees. They are written from AREAS and the message tables: `hatobaSkill.test.ts` fails
 * when a file is out of date, and running it with `-u` writes them again (docs/development.md).
 */

/** The English name of each language that gets label files. */
const LANGUAGE_NAME: Record<Exclude<Locale, "en">, string> = {
  "zh-CN": "Simplified Chinese",
  ja: "Japanese",
};

/** One label file per language: the skill files it covers, each by `## ` section ("" before the first). */
export interface Area {
  name: string;
  /** What the files cover, as the title says it. */
  title: string;
  files: Record<string, Record<string, MessageKey[]>>;
}

export const AREAS: Area[] = [
  {
    name: "app",
    title: "the window, the host list, connecting and the dashboard",
    files: {
      "SKILL.md": {
        "Window layout": [
          "sidebar.settings", "window.toggleSidebar", "dashboard.title", "sidebar.all", "sidebar.favorites",
          "sidebar.recent", "sidebar.groups", "sidebar.newGroup", "sidebar.tags", "sidebar.vault", "sidebar.keys",
          "sidebar.sync", "tray.lock", "tabs.newMenu", "terminal.local.thisComputer", "tabs.newTab", "tabs.close",
          "ai.show", "ai.hide", "window.minimize", "window.maximize", "window.restore", "btn.close", "tray.open",
          "close.tray", "close.quit", "terminal.status.connected", "dashboard.state.connecting",
          "terminal.status.failed", "terminal.status.disconnected", "terminal.find", "hosts.edit.sec.fwd",
          "terminal.sftp", "terminal.more", "settings.tab.general", "settings.tab.appearance", "settings.tab.terminal",
          "settings.tab.proxies", "settings.tab.security", "aiSettings.tab", "settings.tab.about",
        ],
        "Project, feedback and newer information": [
          "terminal.err.copyDiag", "terminal.menu.askAi", "sidebar.star.report", "settings.about.check",
          "settings.about.install", "sidebar.settingsUpdate",
        ],
      },
      "references/hosts-and-connecting.md": {
        "The host list": [
          "sidebar.all", "sidebar.favorites", "hosts.sort.recent", "hosts.search", "hosts.sort", "hosts.sort.name",
          "hosts.sort.address", "hosts.newHost", "hosts.col.tags", "hosts.col.last", "hosts.connect", "hosts.failed",
          "hosts.menu.edit", "hosts.menu.duplicate", "hosts.menu.favorite", "hosts.menu.unfavorite",
          "hosts.menu.copyAddress", "hosts.menu.copyPassword", "hosts.menu.delete", "sidebar.renameGroup",
          "sidebar.deleteGroup", "settings.tab.general", "settings.probe",
        ],
        "Quick connect": [
          "hosts.connect", "hosts.quick.notSaved", "hosts.quick.recent", "hosts.quick.recentTitle",
          "hosts.quick.forget", "terminal.menu.saveAsHost", "hosts.f.user",
        ],
        "Import from ~/.ssh/config": [
          "hosts.empty.import", "hosts.import.selectAll", "hosts.import.selectNone", "hosts.import.exists",
          "hosts.import.proxyCommand",
        ],
        "The host editor": [
          "hosts.editHost", "hosts.edit.sec.basic", "hosts.f.port", "hosts.f.user", "hosts.edit.sec.auth",
          "hosts.f.method", "hosts.edit.sec.org", "hosts.f.group", "hosts.f.jump", "hosts.f.proxy",
          "hosts.edit.sec.fwd", "hosts.edit.sec.env", "hosts.edit.sec.note", "hosts.edit.sec.ai", "hosts.test",
          "hosts.delete", "btn.save", "btn.cancel",
        ],
        "Authentication methods": [
          "hosts.f.password", "hosts.f.key", "hosts.auth.ask", "hosts.auth.agent", "hosts.f.passwordSaved",
          "hosts.f.replace", "hosts.f.keepSaved", "hosts.f.keyPlaceholder", "hosts.f.manageKeys",
        ],
        "Proxy": [
          "hosts.f.proxy", "hosts.f.proxyDefault", "hosts.f.proxyDirect", "hosts.f.proxyDeleted",
          "hosts.f.manageProxies",
        ],
        "Environment variables": [
          "hosts.edit.sec.env", "hosts.env.add",
        ],
        "Connecting, host keys and prompts": [
          "terminal.hostkey.fingerprint", "terminal.hostkey.trust", "terminal.hostkey.changedTitle",
          "terminal.hostkey.known", "terminal.hostkey.received", "terminal.menu.disconnect", "terminal.hostkey.update",
          "terminal.auth.title", "terminal.auth.submit", "terminal.passphrase.title", "terminal.passphrase.label",
          "terminal.hostkey.additionalTitle",
        ],
        "When a connection fails or ends": [
          "terminal.err.copyDiag", "terminal.menu.askAi", "btn.retry", "terminal.disconnected",
          "terminal.menu.reconnect", "terminal.more", "settings.sec.disconnect", "settings.tab.security",
        ],
      },
      "references/dashboard.md": {
        "": [
          "dashboard.title", "dashboard.layout.cards", "dashboard.layout.table", "dashboard.columns",
          "dashboard.sort.list", "dashboard.noGroup", "dashboard.openTerminal", "dashboard.filesystems",
        ],
        "How hosts are read": [
          "dashboard.state.reading", "dashboard.state.offline", "dashboard.state.waiting", "dashboard.viaTab",
          "dashboard.settings.interval", "dashboard.settings.cpuAlert", "dashboard.settings.memAlert",
          "dashboard.settings.diskAlert", "dashboard.settings.off",
        ],
        "Hosts that need the user": [
          "dashboard.hold.password", "dashboard.hold.passphrase", "dashboard.hold.prompt",
          "dashboard.hold.host_key_new", "dashboard.hold.host_key_changed", "dashboard.hold.sign_in_failed",
          "hosts.connect", "btn.retry",
        ],
        "Leaving hosts out": [
          "dashboard.leaveOut", "dashboard.include", "dashboard.leaveOutGroup", "dashboard.includeGroup",
          "dashboard.showLeftOut", "dashboard.leftOutBadge",
        ],
      },
      "references/shortcuts.md": {
        "App shortcuts": [
          "tabs.newTab", "tabs.close", "tray.lock", "sidebar.settings", "ai.show", "ai.hide", "hosts.search",
          "settings.rightClick",
        ],
      },
    },
  },
  {
    name: "terminal",
    title: "terminal tabs, SFTP, port forwarding and connection errors",
    files: {
      "references/terminal.md": {
        "Local terminal tabs": [
          "tabs.newMenu", "terminal.local.title", "tabs.hosts", "terminal.local.thisComputer", "terminal.shell.cmd",
          "terminal.local.status.connecting", "terminal.local.status.connected", "terminal.local.status.failed",
          "terminal.local.status.disconnected", "terminal.local.restart", "settings.term.localShell",
        ],
        "The status bar": [
          "terminal.status.connecting", "terminal.status.connected", "terminal.status.failed",
          "terminal.status.disconnected", "terminal.via", "terminal.stats.show", "terminal.find", "terminal.fwd",
          "terminal.sftp", "terminal.sftpToggle", "terminal.more",
        ],
        "Resource usage": [
          "terminal.stats", "terminal.stats.hide", "terminal.stats.mem", "terminal.stats.swap", "terminal.stats.noSwap",
          "terminal.stats.net", "terminal.stats.disk", "terminal.stats.waiting", "terminal.stats.unsupported",
        ],
        "Menus": [
          "terminal.menu.reconnect", "terminal.menu.disconnect", "terminal.menu.copy", "terminal.menu.paste",
          "terminal.menu.selectAll", "terminal.menu.find", "terminal.menu.askAi", "terminal.menu.clear",
          "terminal.menu.editHost", "terminal.menu.saveAsHost",
        ],
        "Copy, paste, search and links": [
          "terminal.paste.title", "terminal.find.placeholder", "terminal.find.none", "terminal.find.prev",
          "terminal.find.next", "terminal.find.close", "settings.confirmPaste", "settings.tab.general",
          "settings.rightClick", "settings.rightClick.copyPaste", "settings.rightClick.menu",
        ],
        "Terminal settings": [
          "settings.tab.terminal", "sync.field.font_family", "settings.term.size", "settings.term.sizeDown",
          "settings.term.sizeUp", "settings.term.cursor", "settings.term.cursor.block", "settings.term.cursor.bar",
          "settings.term.cursor.underline", "settings.term.scrollback", "settings.tab.appearance",
          "settings.termColors", "settings.termColors.dark", "settings.termColors.light", "settings.termColors.follow",
          "settings.termColors.forDark", "settings.termColors.forLight",
        ],
      },
      "references/sftp.md": {
        "Open it": [
          "sftp.title", "terminal.sftpToggle", "sftp.notConnected",
        ],
        "Browsing": [
          "sftp.upload", "sftp.newFolder", "sftp.refresh", "sftp.parent", "sftp.col.name", "sftp.col.size",
          "sftp.col.modified", "sftp.empty", "sftp.loading", "sftp.loadFailed",
        ],
        "Files and folders": [
          "sftp.menu.open", "sftp.menu.download", "sftp.menu.rename", "sftp.menu.delete", "sftp.menu.copyPath",
          "sftp.menu.newFolder", "sftp.menu.upload", "sftp.invalidName", "sftp.delete.file", "sftp.delete.dir",
          "sftp.pathCopied", "sftp.dropHere",
        ],
        "Transfers": [
          "sftp.transfers", "sftp.cancelTransfer", "sftp.dismissTransfer", "sftp.transferCancelled",
          "sftp.transferDone",
        ],
      },
      "references/port-forwarding.md": {
        "Create or edit a rule": [
          "hosts.edit.sec.fwd", "hosts.fwd.add", "hosts.fwd.dialog.addTitle", "hosts.fwd.dialog.editTitle",
          "hosts.fwd.dialog.add", "hosts.fwd.f.local", "hosts.f.port", "hosts.fwd.f.dest", "hosts.fwd.f.autoStart",
          "hosts.fwd.autoStart", "hosts.fwd.autoPort", "btn.edit", "btn.delete", "btn.save",
        ],
        "Start and stop in a session": [
          "terminal.fwd.start", "terminal.fwd.stop", "btn.retry", "terminal.fwd.manage", "terminal.fwd.none",
          "hosts.fwd.loadFailed",
        ],
      },
      "references/troubleshooting-connections.md": {
        "Reading a failure": [
          "terminal.err.copyDiag", "terminal.menu.askAi", "hosts.editHost", "btn.retry", "hosts.failed", "hosts.test",
          "ai.diagnostics",
        ],
        "Each kind of failure": [
          "ssh.err.auth_failed", "ssh.err.host_key_rejected", "ssh.err.key_parse", "ssh.err.disconnected",
          "ssh.err.protocol", "ssh.err.io", "ssh.err.channel", "ssh.err.sftp", "ssh.err.cancelled", "ssh.err.other",
          "ssh.err.proxy_unreachable", "ssh.err.proxy_auth", "ssh.err.proxy_missing", "err.invalid_input", "err.locked",
          "hosts.sort.address", "hosts.f.port", "hosts.f.user", "hosts.f.method", "hosts.f.password", "hosts.f.replace",
          "hosts.f.key", "hosts.auth.ask", "hosts.auth.agent", "keys.action.deploy",
        ],
        "Jump hosts (ProxyJump)": [
          "hosts.edit.sec.org", "hosts.f.jump", "hosts.err.jumpSelf", "hosts.err.jumpLoop",
        ],
        "Host key prompts and changes": [
          "terminal.hostkey.trust", "btn.cancel", "terminal.hostkey.changedTitle", "terminal.menu.disconnect",
          "terminal.hostkey.update", "terminal.hostkey.additionalTitle",
        ],
        "Dropped connections and keepalive": [
          "terminal.disconnected", "terminal.menu.reconnect", "terminal.status.disconnected", "settings.sec.disconnect",
        ],
      },
    },
  },
  {
    name: "vault",
    title: "keys, the vault, settings and other errors",
    files: {
      "references/keys-vault-and-lock.md": {
        "The vault and the master password": [
          "vault.opt.create", "vault.opt.restore", "vault.step.start", "vault.step.password", "vault.step.recovery",
          "vault.step.connect", "hosts.f.key", "hosts.auth.ask", "vault.code.title", "vault.code.save",
          "vault.code.print", "vault.code.finish", "vault.recover.code", "settings.sec.recovery.button",
          "vault.strength.weak", "vault.strength.fair", "vault.strength.strong", "vault.strength.veryStrong",
        ],
        "Locking and unlocking": [
          "settings.sec.autoLock", "settings.sec.sleep", "settings.sec.disconnect", "settings.sec.lockNow", "tray.lock",
          "settings.sec.never", "settings.sec.hour", "lock.overlay", "vault.unlock.field", "vault.unlock.submit",
          "vault.unlock.forgot", "vault.recover.title", "vault.recover.newPassword", "vault.recover.submit",
          "vault.unlock.hello", "vault.unlock.touchId", "settings.sec.pin", "settings.pin.title", "settings.pin.confirm", "settings.pin.submit",
          "vault.unlock.subtitlePin", "vault.unlock.usePassword", "vault.unlock.usePin", "settings.sec.pinRestart",
          "settings.tab.security", "err.throttled", "lock.sessionsKept",
        ],
        "Master password, backup and clipboard": [
          "settings.sec.clipboard", "vault.field.master", "settings.sec.password.button", "sync.link.password",
          "settings.backup", "settings.backup.button", "settings.tab.general", "hosts.menu.copyPassword",
        ],
        "The Keys page": [
          "sidebar.vault", "sidebar.keys", "keys.search", "keys.import", "keys.generate", "keys.col.name",
          "keys.col.type", "keys.col.fingerprint", "keys.col.created", "keys.col.usedBy", "keys.unused",
          "keys.detail.publicKey", "keys.detail.privateKey", "keys.detail.usedHosts", "keys.detail.lastUsed",
          "keys.detail.copyPublic", "btn.copy", "btn.rename", "keys.action.deploy", "keys.action.delete",
          "keys.import.title", "keys.import.file", "keys.import.paste", "keys.import.chooseFile",
          "keys.field.passphraseOpt", "keys.generate.typeEd25519", "keys.generate.typeRsa", "keys.generate.comment",
          "keys.generate.confirmPassphrase", "keys.rename.title", "keys.deploy.title", "keys.deploy.host",
          "keys.deploy.action", "keys.delete.confirm", "keys.import.action", "keys.generate.action",
        ],
      },
      "references/settings.md": {
        "": [
          "settings.title", "settings.tab.general", "settings.tab.appearance", "settings.tab.terminal",
          "settings.tab.proxies", "settings.tab.security", "aiSettings.tab", "settings.tab.about",
        ],
        "General": [
          "settings.rightClick", "settings.rightClick.copyPaste", "settings.rightClick.menu", "settings.confirmPaste",
          "settings.probe", "settings.closeAction", "settings.closeAction.ask", "settings.closeAction.tray",
          "settings.closeAction.quit", "settings.aiNotifications", "dashboard.settings.interval", "settings.backup",
          "settings.backup.button",
        ],
        "Appearance": [
          "settings.appearance.light", "settings.appearance.dark", "settings.appearance.auto", "settings.termColors",
          "settings.termColors.dark", "settings.termColors.light", "settings.termColors.follow", "settings.density",
          "settings.density.regular", "settings.density.compact", "settings.language", "settings.language.system",
          "settings.language.zh", "settings.language.en", "settings.language.ja",
        ],
        "Proxies": [
          "settings.proxy.default", "settings.proxy.none", "settings.proxy.saved", "settings.proxy.add",
          "settings.proxy.dlg.editTitle", "settings.proxy.f.kind", "settings.proxy.isDefault",
        ],
        "Security": [
          "settings.sec.autoLock", "settings.sec.sleep", "settings.sec.disconnect", "settings.sec.clipboard",
          "settings.sec.hello", "settings.sec.touchId", "settings.sec.pin", "settings.sec.pinRestart", "settings.sec.password",
          "settings.sec.password.button", "settings.sec.recovery", "settings.sec.recovery.button",
          "settings.sec.lockNow", "settings.sec.lockNow.button",
        ],
        "About": [
          "settings.about.update", "settings.about.check", "settings.about.install", "settings.about.install.confirm",
          "settings.about.releasePage", "settings.about.auto", "settings.about.checking", "settings.about.latest",
          "settings.about.copyVersion", "settings.about.website", "settings.about.source", "settings.about.license",
          "settings.about.report", "sidebar.settingsUpdate", "sidebar.star.title", "sidebar.star.star", "tray.update",
          "settings.about.available",
        ],
      },
      "references/troubleshooting.md": {
        "Key errors": [
          "key.err.unsupported_format", "key.err.passphrase_required", "key.err.wrong_passphrase",
          "key.err.unsupported_algorithm", "key.err.invalid", "err.key_parse", "hosts.auth.ask",
        ],
        "Locked, password and recovery": [
          "err.locked", "err.wrong_password", "err.wrong_recovery_code", "vault.err.mismatch", "vault.err.tooShort",
          "vault.err.tooWeak", "err.not_initialized", "err.already_initialized", "vault.unlock.forgot",
          "settings.sec.recovery.button", "err.throttled",
        ],
        "Files, forwards and other messages": [
          "err.sftp", "err.io", "err.internal", "err.not_found", "err.cancelled", "terminal.clipboardDenied",
          "sftp.loadFailed", "sftp.invalidName", "hosts.edit.sec.fwd",
        ],
        "Updates": [
          "settings.about.offline", "settings.about.failed", "settings.about.installFailed", "err.update_signature",
          "settings.about.check", "settings.about.install", "settings.about.releasePage",
        ],
        "Still not solved": [
          "sidebar.star.report", "settings.tab.about",
        ],
      },
    },
  },
  {
    name: "sync",
    title: "Cloud Sync and the sync Worker",
    files: {
      "references/sync.md": {
        "": [
          "sidebar.sync", "sidebar.vault", "sync.footer.none", "sync.footer.none_sub", "sync.footer.synced",
          "sync.footer.syncing", "sync.footer.offline", "sync.footer.offline_sub_none", "sync.footer.auth",
          "sync.footer.auth_sub", "sync.footer.error", "sync.footer.workerUpdate", "sync.footer.appUpdate",
          "sync.footer.conflict_sub",
        ],
        "Set up on the first device": [
          "sync.step.method", "sync.step.connect", "sync.step.password", "sync.opt.deploy", "sync.opt.recommended",
          "sync.opt.worker", "sync.opt.token", "sync.howEncrypt", "btn.continue", "btn.cancel", "btn.back",
          "sync.w3.title", "sync.masterPassword", "sync.finish", "sync.enabled", "sync.uploading",
        ],
        "Join a cloud vault": [
          "sync.test.hasVault", "sync.join.title", "sync.join.submit", "sync.join.busy",
        ],
        "Add another device": [
          "vault.opt.restore", "vault.restore.method", "sync.method.worker", "sync.method.d1", "sync.workerUrl",
          "hosts.test", "vault.restore.submit",
        ],
        "The status page": [
          "sync.syncNow", "sync.sec.connection", "sync.row.endpoint", "sync.database", "sync.row.auto",
          "sync.sec.devices", "sync.device.thisPc", "sync.device.stale", "btn.remove", "sync.auth.login",
          "sync.device.remove.confirm",
        ],
        "Conflicts": [
          "sync.conflictBanner.view", "sync.cf.keep", "sync.cf.restore", "sync.cf.cloud", "sync.cf.empty",
          "sync.cf.result.kept_both",
        ],
        "Security actions and disconnecting": [
          "sync.field.always_ask", "sync.link.recovery", "sync.link.password", "sync.link.disconnect",
          "sync.disconnect.title", "sync.disconnect.confirm",
        ],
      },
      "references/sync-worker.md": {
        "Method 1: Deploy a Worker from Hatoba": [
          "sync.step.connect", "sync.dep.title", "sync.dep.token.title", "sync.dep.createToken", "sync.apiToken",
          "sync.dep.accountId", "sync.dep.review.title", "sync.dep.account", "sync.dep.subdomain", "sync.dep.names",
          "sync.dep.workerName", "sync.dep.databaseName", "sync.dep.deploy", "btn.continue", "sync.dep.step.verify",
          "sync.dep.step.inspect", "sync.dep.step.create_database", "sync.dep.step.migrate", "sync.dep.step.upload",
          "sync.dep.step.setup_token", "sync.dep.step.route", "sync.dep.step.wait", "sync.dep.skipped",
          "sync.dep.run.ready", "sync.dep.run.failed", "btn.retry", "sync.dep.remove", "sync.dep.checkAgain",
          "btn.back",
        ],
        "Method 2: Connect a deployed Worker": [
          "sync.w2.title", "sync.oneClick", "sync.deploy", "sync.wrangler", "sync.wrangler.guide", "sync.workerUrl",
          "sync.setupToken", "sync.test", "sync.test.connected",
        ],
        "Method 3: Use an API Token (direct D1)": [
          "sync.d1.title", "sync.accountId", "sync.d1.verify", "sync.database", "sync.database.choose",
          "sync.token.edit",
        ],
        "Upgrading the Worker": [
          "sync.wu.update", "sync.wu.dismiss", "sync.wu.checkApp", "sync.up.title", "sync.up.review.title",
          "sync.up.update", "sync.up.run.title", "sync.up.run.ready", "sync.up.run.failed", "sync.up.step.inspect",
          "sync.up.step.migrate", "sync.up.guide", "sync.footer.workerUpdate", "sync.footer.appUpdate",
        ],
      },
      "references/troubleshooting-sync.md": {
        "Sync states": [
          "sync.footer.offline", "sync.footer.offline_sub_none", "sync.footer.auth", "sync.footer.auth_sub",
          "sync.footer.error", "sync.footer.workerUpdate", "sync.footer.appUpdate", "sync.footer.none",
          "sync.footer.none_sub", "sync.auth.login", "sync.syncNow", "sync.row.endpoint", "sync.wu.update",
          "settings.about.check",
        ],
        "Messages": [
          "err.sync", "err.sync_offline", "err.sync_auth", "err.remote_initialized", "err.remote_not_initialized",
          "err.sync_setup_busy", "err.sync_already_configured", "sync.test.failed", "vault.restore.badUrl",
          "err.wrong_password", "err.cancelled",
        ],
        "An item too large to sync": [
          "sync.tooLarge.settings", "hosts.err.tooLarge", "hosts.editHost", "ai.noProvider.open",
          "aiSettings.instructions", "sync.tooLarge.host",
        ],
        "Cloudflare and deployment errors": [
          "err.cloudflare_token", "err.cloudflare", "err.subdomain_required", "err.subdomain_unavailable",
          "err.worker_name_taken", "err.worker_not_found", "err.worker_newer", "err.no_worker_bundle", "sync.token.err",
          "sync.dep.offline", "sidebar.sync", "vault.opt.restore", "sync.opt.deploy", "sync.opt.worker",
          "sync.accountId", "sync.dep.subdomain", "sync.dep.checkAgain", "sync.dep.remove", "sync.dep.step.verify",
          "sync.dep.step.inspect", "sync.dep.step.create_database", "sync.dep.step.migrate", "sync.dep.step.upload",
          "sync.dep.step.setup_token", "sync.dep.step.route", "sync.dep.step.wait", "sync.dep.createToken",
          "sync.token.edit", "sync.dep.accountId", "sync.dep.names", "btn.retry",
        ],
        "Devices and conflicts": [
          "sync.conflictBanner",
        ],
        "Still not solved": [
          "sidebar.star.report", "sync.conflictBanner.view", "sync.sec.devices", "btn.remove",
        ],
      },
    },
  },
  {
    name: "ai",
    title: "the AI panel, its tools and conversations",
    files: {
      "references/ai-panel.md": {
        "Opening the panel": [
          "ai.title", "ai.show", "ai.hide", "ai.close", "ai.noProvider.open", "terminal.menu.askAi", "terminal.more",
          "settings.rightClick", "settings.rightClick.menu",
        ],
        "One conversation per tab": [
          "ai.newConversation", "ai.history", "ai.model", "ai.effort", "ai.compact", "ai.send", "ai.stop",
          "ai.stopAndSend", "ai.queued", "ai.queued.edit", "ai.queued.remove", "ai.add", "ai.add.slash",
          "ai.add.skills", "ai.tools", "ai.tabRunning", "ai.tabWaiting", "terminal.menu.reconnect", "ai.continueHere",
          "ai.connectTo", "ai.note.hostChange",
        ],
        "`/` commands and `@` mentions": [
          "ai.cmd.menu", "ai.mention.menu", "ai.mention.local",
        ],
        "MCP servers: **MCP Tools** and `/mcp`": [
          "ai.tools.title", "ai.tools.manage", "ai.tools.off", "ai.tools.stopped", "ai.tools.starting",
          "ai.tools.failed", "ai.status.mcpStarting", "ai.note.mcpTools",
        ],
      },
      "references/ai-tools.md": {
        "Tools": [
          "ai.tool.read_terminal", "ai.tool.run_command", "ai.tool.send_input", "ai.tool.read_file",
          "ai.tool.edit_file", "ai.tool.write_file", "ai.tool.grep", "ai.tool.glob", "ai.tool.web_search",
          "ai.tool.fetch_url", "ai.tool.read_skill", "ai.tool.search_tools", "ai.tool.memory",
        ],
        "Permission mode": [
          "ai.mode", "ai.mode.manual", "ai.mode.auto", "ai.mode.autoShort", "ai.mode.bypass", "ai.mode.bypassShort",
          "ai.bypass.confirm", "ai.note.permissionBypass", "ai.note.permissionAuto", "ai.note.permissionManual",
          "ai.mode.autoNeedsJudge", "ai.mode.chooseJudge", "ai.call.judging", "aiSettings.tab", "aiSettings.device",
          "aiSettings.mode", "aiSettings.default", "aiSettings.mcp.alwaysAsk", "ai.auto.failure.unavailable",
          "ai.auto.failure.too_long", "ai.auto.failure.timeout", "ai.auto.failure.unreachable",
          "ai.auto.failure.refused", "ai.auto.failure.malformed",
        ],
        "Approval cards": [
          "ai.call.approval", "ai.approval.workdir", "ai.approval.homeDir", "ai.approval.run", "ai.queued.edit",
          "ai.approval.reject", "ai.approval.allowHere", "ai.approval.alwaysAllow", "ai.approval.alwaysAllowHost",
          "ai.approval.reasonPlaceholder", "ai.approval.escaped", "ai.approval.alwaysAllowCommand",
        ],
      },
      "references/ai-history.md": {
        "History, search, export": [
          "ai.search", "ai.history.open", "ai.history.fork", "btn.rename", "ai.history.pin", "ai.history.unpin",
          "ai.history.export", "ai.history.delete", "ai.history.more",
        ],
        "Edit and Resend": [
          "ai.edit", "ai.edit.send",
        ],
        "Fork a conversation": [
          "ai.fork", "ai.fork.done", "ai.fork.inHistory", "ai.approval.allowHere",
        ],
      },
      "references/ai-models.md": {
        "The model": [
          "ai.model", "aiSettings.default", "aiSettings.default.row", "ai.note.modelChange",
        ],
        "Thinking level": [
          "ai.effort", "ai.effort.default", "ai.effort.low", "ai.effort.medium", "ai.effort.high", "ai.effort.xhigh",
          "ai.effort.max", "ai.reasoning",
        ],
        "Context and compaction": [
          "ai.meter", "ai.compact", "ai.compact.menu", "ai.outsideContext", "aiSettings.models.col.context", "ai.stop",
        ],
      },
      "references/ai-file-changes.md": {
        "": [
          "ai.revert.action", "ai.revert.reverted", "ai.revert.delete", "ai.editFiles.revert", "ai.editFiles.keep",
          "ai.editFiles.sendAnyway", "ai.revert.restore", "ai.revert.deleteAnyway",
        ],
        "Reverting": [
          "ai.note.fileRevert",
        ],
      },
      "references/ai-attachments.md": {
        "Terminal selection": [
          "terminal.menu.askAi", "ai.selection.preview", "ai.selection.remove", "ai.selection.truncated",
        ],
        "Asking about a failed connection": [
          "terminal.err.copyDiag", "hosts.editHost", "btn.retry", "ai.diagnostics.preview", "ai.diagnostics.sent",
          "ai.diagnostics.remove", "ai.connection.question", "ai.diagnostics",
        ],
        "Long pastes": [
          "ai.paste.preview", "ai.paste.asText", "ai.paste.remove", "ai.paste",
        ],
        "Text files": [
          "ai.add", "ai.attach", "ai.attach.drop", "ai.file.preview", "ai.attach.tooLarge", "ai.attach.total",
        ],
        "The terminal's screen": [
          "ai.note.screen.fullScreen", "aiSettings.screen", "ai.screen", "ai.screen.noTerminal",
          "aiSettings.screen.row", "ai.note.screen",
        ],
        "Does it fit the model?": [
          "ai.fit.blocked", "aiSettings.models.col.context",
        ],
        "After sending": [
          "ai.edit", "ai.history.export",
        ],
      },
      "references/ai-side-questions.md": {
        "Asking one": [
          "ai.side.ask", "ai.side.askCard", "ai.side.send",
        ],
        "The answer": [
          "ai.side.title", "ai.side.notSaved",
        ],
      },
    },
  },
  {
    name: "ai-settings",
    title: "AI settings, providers, MCP servers and AI errors",
    files: {
      "references/ai-instructions.md": {
        "": [
          "aiSettings.instructions", "hosts.edit.sec.ai", "ai.tool.memory", "aiSettings.memory.row",
          "aiSettings.memory.add", "aiSettings.memory.deleteAll",
        ],
      },
      "references/ai-settings.md": {
        "": [
          "settings.title", "aiSettings.tab", "aiSettings.providers", "aiSettings.default", "aiSettings.search",
          "aiSettings.instructions", "aiSettings.screen", "ai.tool.memory", "ai.add.skills", "aiSettings.mcp",
          "aiSettings.judge", "aiSettings.device",
        ],
        "Providers": [
          "aiSettings.providers.add", "settings.proxy.f.name", "aiSettings.f.protocol",
          "aiSettings.protocol.chat_completions", "aiSettings.protocol.responses", "aiSettings.protocol.anthropic",
          "aiSettings.protocol.system_one", "aiSettings.f.baseUrl", "aiSettings.f.apiKey", "aiSettings.f.authHeader",
          "aiSettings.test", "btn.save", "aiSettings.f.apiKeySaved", "aiSettings.f.apiKeyReplace",
          "aiSettings.f.apiKeyClear", "aiSettings.models", "aiSettings.models.fetch", "aiSettings.models.col.id",
          "aiSettings.models.col.name", "aiSettings.models.col.context", "aiSettings.models.col.output",
          "aiSettings.models.col.effort", "aiSettings.models.effort.menu", "aiSettings.models.effort.none",
          "aiSettings.models.add",
        ],
        "Default Model and Web Search": [
          "aiSettings.default.row", "aiSettings.default.effort", "aiSettings.search.row", "aiSettings.search.native",
          "aiSettings.search.brave", "aiSettings.search.tavily", "aiSettings.search.searxng",
          "aiSettings.search.baseUrl", "aiSettings.search.remove",
        ],
        "Terminal Screen": [
          "aiSettings.screen.row",
        ],
        "Skills": [
          "ai.tool.read_skill", "aiSettings.skills.add", "aiSettings.skills.import", "aiSettings.skills.import.folder",
          "aiSettings.skills.import.zip", "aiSettings.skills.f.description", "aiSettings.skills.f.body",
          "aiSettings.skills.files", "aiSettings.skills.files.add", "aiSettings.skills.imp.title",
          "aiSettings.skills.imp.files", "aiSettings.skills.imp.replace", "aiSettings.skills.imp.rename",
          "aiSettings.skills.imp.submitReplace", "aiSettings.skills.builtin", "aiSettings.skills.builtin.title",
          "aiSettings.skills.err.nameReserved", "aiSettings.skills.reservedNote", "aiSettings.skills.builtin.view",
        ],
        "Auto Mode": [
          "aiSettings.judge", "aiSettings.judge.row", "aiSettings.judge.placeholder", "aiSettings.judge.test",
          "aiSettings.judge.test.details", "aiSettings.judge.decision.run", "aiSettings.judge.decision.ask",
          "aiSettings.judge.test.match", "aiSettings.judge.test.mismatch", "aiSettings.judge.test.unanswered",
        ],
        "This Device": [
          "aiSettings.mode", "ai.mode.manual", "ai.mode.auto", "ai.mode.bypass", "aiSettings.bypass.confirm",
          "aiSettings.allowances", "btn.remove",
        ],
      },
      "references/ai-chatgpt.md": {
        "": [
          "aiSettings.chatgpt.continue", "aiSettings.chatgpt.kind", "aiSettings.chatgpt.account",
          "aiSettings.chatgpt.signOut", "aiSettings.chatgpt.manageUsage", "aiSettings.chatgpt.waiting.title",
          "aiSettings.chatgpt.welcome.title", "aiSettings.chatgpt.status.signed_in",
          "aiSettings.chatgpt.row.signed_out", "aiSettings.chatgpt.row.ended", "aiSettings.chatgpt.row.plan_disabled",
        ],
        "Problems": [
          "aiSettings.chatgpt.failed.title", "aiSettings.chatgpt.test.auth", "aiSettings.chatgpt.signedOutUnconfirmed",
          "aiSettings.chatgpt.status.plan_disabled",
        ],
      },
      "references/ai-mcp.md": {
        "": [
          "aiSettings.mcp.add", "aiSettings.mcp.templates", "aiSettings.mcp.export", "aiSettings.mcp.dlg.addTitle",
          "aiSettings.mcp.f.transport", "ai.approval.command", "aiSettings.mcp.kind.http", "ai.approval.arguments",
          "aiSettings.mcp.f.split", "aiSettings.mcp.f.env", "aiSettings.mcp.f.url", "aiSettings.mcp.f.headers",
          "aiSettings.mcp.alwaysAsk", "aiSettings.mcp.imp.title", "aiSettings.mcp.imp.preview",
          "aiSettings.mcp.state.stopped", "aiSettings.mcp.state.starting", "ai.call.running",
          "aiSettings.mcp.state.failed", "aiSettings.mcp.state.off", "aiSettings.mcp.start", "aiSettings.mcp.restart",
          "ai.stop", "aiSettings.mcp.allowAll", "aiSettings.mcp.allowTool", "aiSettings.mcp.tools",
          "aiSettings.mcp.stderr",
        ],
      },
      "references/troubleshooting-ai.md": {
        "The provider returned an error": [
          "ai.outcome.length", "ai.outcome.refused", "ai.outcome.empty", "ai.outcome.error", "ai.emptyReply",
          "ai.err.noModel", "ai.loadFailed", "ai.history.failed", "ai.search.failed", "ai.stop", "btn.retry",
          "ai.compact", "aiSettings.f.authHeader", "aiSettings.f.apiKeyReplace", "aiSettings.models.fetch",
          "aiSettings.models.col.context", "aiSettings.models.col.output", "ai.newConversation", "aiSettings.f.baseUrl",
          "aiSettings.models", "ai.outcome.errorStatus",
        ],
        "Thinking level and reasoning": [
          "ai.effort", "ai.effort.default", "ai.effortIgnored", "ai.reasoning", "aiSettings.models.col.effort",
          "aiSettings.models.effort.menu", "aiSettings.models.effort.none", "aiSettings.default.effort",
        ],
        "Test Connection and provider settings": [
          "aiSettings.test", "aiSettings.test.fail.auth", "aiSettings.test.fail.network",
          "aiSettings.test.fail.invalid_url", "aiSettings.err.provider", "aiSettings.err.urlInvalid",
          "aiSettings.err.urlRejected", "aiSettings.err.keyRequired", "aiSettings.models.fetchFailed",
          "aiSettings.models.err.tokens", "aiSettings.models.err.idEmpty",
        ],
        "Web search": [
          "aiSettings.search.test.fail.auth", "aiSettings.search.test.fail.network",
          "aiSettings.search.test.fail.invalid_url", "aiSettings.search.test.fail.other",
          "aiSettings.search.err.urlRequired", "aiSettings.search", "aiSettings.search.searxng",
        ],
        "Skills": [
          "aiSettings.skills.err.nameInvalid", "aiSettings.skills.err.nameReserved", "aiSettings.skills.reservedNote",
          "aiSettings.skills.builtin", "aiSettings.skills.err.nameTaken", "aiSettings.skills.err.descRequired",
          "aiSettings.skills.err.bodyTooLarge", "aiSettings.skills.err.tooLarge",
          "aiSettings.skills.issue.missing_skill_md", "aiSettings.skills.imp.failed",
          "aiSettings.skills.imp.issuesTitle", "aiSettings.skills.imp.skipped",
        ],
        "MCP servers": [
          "aiSettings.mcp.status.failed", "aiSettings.mcp.state.failed", "aiSettings.mcp.stderr",
          "aiSettings.mcp.err.commandRequired", "aiSettings.mcp.err.urlRejected", "aiSettings.mcp.f.commandLine",
          "aiSettings.mcp.f.split", "aiSettings.mcp.f.splitFailed", "aiSettings.mcp.imp.none", "aiSettings.mcp.start",
          "aiSettings.mcp.restart",
        ],
        "Still not solved": [
          "sidebar.star.report", "aiSettings.skills.imp.replace", "aiSettings.skills.imp.rename", "ai.approval.command",
          "aiSettings.mcp.f.headers", "aiSettings.mcp", "aiSettings.tab",
        ],
      },
    },
  },
];

export interface LabelFile {
  /** The path inside the skill. */
  path: string;
  text: string;
}

/** Every label file of the skill, for each language but English and each area. */
export function labelFiles(): LabelFile[] {
  const files: LabelFile[] = [];
  for (const [locale, language] of Object.entries(LANGUAGE_NAME) as [Exclude<Locale, "en">, string][]) {
    const table = messages[locale];
    for (const area of AREAS) {
      const parts = [
        `# ${language} labels: ${area.title}`,
        `The labels that these files name in English, as the interface shows them in ${language} (${locale}). Quote them exactly; the same English label can read differently in another file or section. A word in braces, such as {host}, stands for the value the interface fills in.`,
      ];
      for (const [file, sections] of Object.entries(area.files)) {
        parts.push(`## \`${file}\``);
        for (const [heading, keys] of Object.entries(sections)) {
          if (heading) parts.push(`### ${heading}`);
          // The text goes in as it is, since the assistant quotes it: the test keeps out a "|", which would end the cell.
          const rows = keys.map((key) => `| ${messages.en[key]} | ${table[key]} |`);
          parts.push([`| en | ${locale} |`, "|---|---|", ...rows].join("\n"));
        }
      }
      files.push({ path: `references/labels/${locale}-${area.name}.md`, text: `${parts.join("\n\n")}\n` });
    }
  }
  return files;
}
