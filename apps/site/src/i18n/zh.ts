import type { Messages } from "./en";

export const zh: Messages = {
  meta: {
    description: "开源桌面 SSH 客户端，内置 AI 助手，并通过你自己的 Cloudflare 账户进行端到端加密同步",
    tagline: "内置 AI 与加密同步的开源 SSH 客户端",
    downloadTitle: "下载 Hatoba",
    downloadLead: (version) => `下载适用于 Windows、Linux 和 macOS 的 Hatoba ${version}。`,
  },
  nav: {
    home: "Hatoba 首页",
    features: "功能",
    download: "下载",
    docs: "文档",
    github: "GitHub",
    language: "语言",
    skip: "跳到正文",
  },
  platforms: { windows: "Windows", linux: "Linux", macos: "macOS" },
  hero: {
    downloadFor: (platform) => `下载 ${platform} 版`,
    allDownloads: "全部下载",
    github: "在 GitHub 上查看",
    available: (version) => `版本 ${version}，支持 Windows、Linux 和 macOS`,
    alt: "Hatoba 的 AI 助手在终端旁找出并修复了 nginx 502 错误",
  },
  ai: {
    title: "每个终端旁都有 AI 助手",
    body: "在终端旁的面板里描述问题，AI 助手会读取屏幕、运行命令、读取和编辑主机上的文件，找出原因并修好它。",
    points: [
      "每条命令都要等你批准，运行前还可以修改。开启自动批准后，由你选择的模型批准其中安全的操作。",
      "它改动的每个文件都会显示差异，可以撤销。",
      "输入 / 使用命令和技能，输入 @ 附加主机上的文件。还可以添加自己的技能和 MCP 服务器。",
      "可以使用任何兼容 OpenAI 或 Anthropic API 的模型，包括本地模型。",
    ],
    alt: "AI 助手请求运行一条命令时的批准卡片",
  },
  sync: {
    title: "通过你自己的 Cloudflare 账户同步",
    body: "Hatoba 会把 Worker 和 D1 数据库部署到你的 Cloudflare 账户，然后在你的设备之间同步主机、密钥和 AI 对话。个人使用通常在免费套餐的额度之内。",
    points: [
      "端到端加密：数据离开设备前就已加密，Worker 和数据库里只有密文。",
      "没有 Hatoba 服务器：同步完全在你的账户里运行。",
      "同步是可选的：不开启它，所有功能也都能离线使用。",
    ],
    alt: "云同步页面，显示连接方式、设备列表和加密说明",
  },
  essentials: {
    title: "SSH 客户端该有的，这里都有",
    items: [
      {
        term: "主机",
        text: "分组、标签、收藏和模糊搜索。可以导入 ~/.ssh/config，通过 ProxyJump 或 SOCKS5、HTTP 代理连接，也可以直接输入 ssh user@host 连接，不必先保存主机。",
      },
      {
        term: "仪表盘",
        text: "在一个页面上查看所有已保存主机的状态、CPU、内存、磁盘和网络。数据通过 SSH 读取，服务器上无需安装任何东西。",
      },
      {
        term: "终端",
        text: "多标签、真彩色、中日韩输入法、搜索和可点击的链接，粘贴多行内容前会先确认。",
      },
      { term: "SFTP", text: "终端旁的文件面板，支持拖放上传，下载可以随时取消。" },
      { term: "端口转发", text: "本地转发，可以随连接一起启动。" },
      {
        term: "密钥",
        text: "支持密码、Ed25519、ECDSA 和 RSA 密钥、ssh-agent 以及键盘交互式双重验证。可以导入 OpenSSH、PEM 和 PuTTY 密钥，也可以生成新密钥。",
      },
    ],
    alt: "仪表盘，显示每台主机的 CPU、内存和磁盘使用情况",
  },
  security: {
    title: "在你的设备上加密",
    body: "主密码用来解锁你的保险库。密钥由主密码经 Argon2id 派生，每一项数据在存储或同步之前都会用 AES-256-GCM 加密。如果忘记主密码又丢失了恢复码，数据将无法恢复。",
    openSource: "Hatoba 以 MIT 许可证开源，你可以亲自查看它如何处理你的数据。",
    link: "阅读安全模型（英文）",
  },
  cta: {
    title: "开始使用 Hatoba",
    body: "免费开源，支持 Windows 10 和 11、x86_64 架构的 Linux，以及搭载 Apple 芯片的 Mac（实验性）。",
  },
  footer: {
    releases: "版本发布",
    report: "报告问题",
    security: "安全策略",
    contributing: "参与贡献",
    license: "Hatoba 以 MIT 许可证发布。",
  },
  download: {
    released: (version, date) => `版本 ${version}，发布于 ${date}。`,
    notes: "发布说明",
    preview: (version) => `${version} 预览版已经发布。`,
    previewLink: "查看预览版",
    thisDevice: "适用于此设备",
    requirements: {
      windows: "Windows 10 或 11（x64）。",
      linux: "x86_64。同步把凭据保存在 Secret Service 中，因此需要 GNOME Keyring、KWallet 或其他 Secret Service 提供程序。",
      macos: "Apple 芯片，macOS 13 或更高版本。此版本为实验性版本。",
    },
    files: {
      nsis: "安装程序（.exe）",
      deb: "Debian 和 Ubuntu（.deb）",
      appimage: "其他发行版（AppImage）",
      dmg: "磁盘映像（.dmg）",
    },
    size: (megabytes) => `${megabytes} MB`,
    install: "安装",
    sha256: "SHA-256",
    copy: "复制",
    copied: "已复制",
    checksums: "列出了这个版本中每个文件的校验和。",
    allReleases: "所有版本",
    source: "从源码构建",
    testing: "Hatoba 主要在 Windows 上开发，Linux 和 macOS 版本的测试较少。如果遇到问题，欢迎提交 issue。",
    issue: "提交 issue",
  },
  notFound: {
    title: "找不到页面",
    body: "这个地址上没有页面。",
    home: "返回首页",
  },
};
