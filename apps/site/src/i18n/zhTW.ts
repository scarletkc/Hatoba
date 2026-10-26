import type { Messages } from "./en";

export const zhTW: Messages = {
  meta: {
    description: "開源桌面 SSH 用戶端，內建 AI 助理，並透過你自己的 Cloudflare 帳戶進行端對端加密同步",
    tagline: "內建 AI 與加密同步的開源 SSH 用戶端",
    downloadTitle: "下載 Hatoba",
    downloadLead: (version) => `下載適用於 Windows、Linux 和 macOS 的 Hatoba ${version}。`,
  },
  nav: {
    home: "Hatoba 首頁",
    features: "功能",
    download: "下載",
    docs: "文件",
    github: "GitHub",
    language: "語言",
    skip: "跳到主要內容",
  },
  platforms: { windows: "Windows", linux: "Linux", macos: "macOS" },
  hero: {
    downloadFor: (platform) => `下載 ${platform} 版`,
    allDownloads: "全部下載",
    github: "在 GitHub 上檢視",
    available: (version) => `版本 ${version}，支援 Windows、Linux 和 macOS`,
    alt: "Hatoba 的 AI 助理在終端機旁找出並修好了 nginx 502 錯誤",
  },
  demo: {
    try: "在瀏覽器裡試用",
    title: "Hatoba 示範",
    note: "示範使用範例資料：不會連線到任何地方，助理的回覆也是預先寫好的。",
    restart: "重新開始",
    newTab: "在新分頁開啟",
    close: "關閉示範",
  },
  ai: {
    title: "每個終端機旁都有 AI 助理",
    body: "在終端機旁的面板裡描述問題，AI 助理會讀取螢幕、執行命令、讀取和編輯主機上的檔案，找出原因並修好它。",
    points: [
      "每個命令都要等你核准，執行前還可以修改。開啟自動核准後，由你選擇的模型核准其中安全的操作。",
      "它變更的每個檔案都會顯示差異，可以復原。",
      "輸入 / 使用命令和技能，輸入 @ 附加主機上的檔案。還可以新增自己的技能和 MCP 伺服器。",
      "可以使用任何相容 OpenAI 或 Anthropic API 的模型，包括本機模型。",
    ],
    alt: "AI 助理要求執行一個命令時的核准卡片",
  },
  sync: {
    title: "透過你自己的 Cloudflare 帳戶同步",
    body: "Hatoba 會把 Worker 和 D1 資料庫部署到你的 Cloudflare 帳戶，然後在你的裝置之間同步主機、金鑰和 AI 對話。個人使用通常在免費方案的額度之內。",
    points: [
      "端對端加密：資料離開裝置前就已加密，Worker 和資料庫裡只有密文。",
      "沒有 Hatoba 伺服器：同步完全在你的帳戶裡執行。",
      "同步是選用的：不開啟它，所有功能也都能離線使用。",
    ],
    alt: "雲端同步頁面，顯示連線方式、裝置清單和加密說明",
  },
  essentials: {
    title: "SSH 用戶端該有的，這裡都有",
    items: [
      {
        term: "主機",
        text: "群組、標籤、我的最愛和模糊搜尋。可以匯入 ~/.ssh/config，透過 ProxyJump 或 SOCKS5、HTTP 代理連線，也可以直接輸入 ssh user@host 連線，不必先儲存主機。",
      },
      {
        term: "資源使用量",
        text: "直接在主機頁查看所有已儲存主機的狀態、CPU、記憶體、磁碟和網路。資料透過 SSH 讀取，伺服器上不必安裝任何東西。",
      },
      {
        term: "終端機",
        text: "多分頁、真彩色、中日韓輸入法、搜尋和可點擊的連結，貼上多行內容前會先確認。",
      },
      {
        term: "SFTP",
        text: "終端機旁的檔案面板，支援拖放上傳，下載可以隨時取消，遠端檔案可以用你自己的編輯器編輯，每次儲存都會上傳。",
      },
      { term: "連接埠轉送", text: "本機、遠端和 SOCKS 轉送，可以隨連線一起啟動，也可以不開終端機在背景執行。" },
      {
        term: "金鑰",
        text: "支援密碼、Ed25519、ECDSA 和 RSA 金鑰、ssh-agent 以及鍵盤互動式雙重驗證。可以匯入 OpenSSH、PEM 和 PuTTY 金鑰，也可以產生新金鑰。",
      },
    ],
    alt: "卡片檢視的主機頁，顯示每台主機的 CPU、記憶體和磁碟使用情形",
  },
  security: {
    title: "在你的裝置上加密",
    body: "主密碼用來解鎖你的保險庫。金鑰由主密碼經 Argon2id 衍生，每一項資料在儲存或同步之前都會用 AES-256-GCM 加密。如果忘記主密碼又遺失了復原碼，資料將無法復原。",
    openSource: "Hatoba 以 MIT 授權條款開源，你可以親自查看它如何處理你的資料。",
    link: "閱讀安全性模型（英文）",
  },
  cta: {
    title: "開始使用 Hatoba",
    body: "免費開源，支援 Windows 10 和 11、x86_64 架構的 Linux，以及搭載 Apple 晶片的 Mac（實驗性）。",
  },
  footer: {
    releases: "版本發布",
    report: "回報問題",
    security: "安全性政策",
    privacy: "隱私權政策",
    contributing: "參與貢獻",
    license: "Hatoba 以 MIT 授權條款發布。",
  },
  download: {
    released: (version, date) => `版本 ${version}，發布於 ${date}。`,
    notes: "版本說明",
    preview: (version) => `${version} 預覽版已經發布。`,
    previewLink: "查看預覽版",
    thisDevice: "適用於這台裝置",
    requirements: {
      windows: "Windows 10 或 11（x64）。",
      linux:
        "x86_64。同步把憑證儲存在 Secret Service 中，因此需要 GNOME Keyring、KWallet 或其他 Secret Service 提供者。",
      macos: "Apple 晶片，macOS 13 或更新版本。此版本為實驗性版本。",
    },
    files: {
      nsis: "安裝程式（.exe）",
      deb: "Debian 和 Ubuntu（.deb）",
      appimage: "其他發行版（AppImage）",
      dmg: "磁碟映像檔（.dmg）",
    },
    size: (megabytes) => `${megabytes} MB`,
    install: "安裝",
    packageManager: "使用套件管理器",
    packages: {
      windows: "從 [Scoop](https://scoop.sh) 的 Extras bucket 安裝 Hatoba，之後用 `scoop update hatoba` 更新：",
      linux:
        "在 Arch Linux 及其衍生發行版上，用 yay 等 AUR 輔助工具從 AUR 安裝 [`hatoba-bin`](https://aur.archlinux.org/packages/hatoba-bin)，之後它會隨系統一起更新：",
    },
    sha256: "SHA-256",
    copy: "複製",
    copied: "已複製",
    checksums: "列出了這個版本中每個檔案的檢查碼。",
    allReleases: "所有版本",
    source: "從原始碼建置",
    testing: "Hatoba 主要在 Windows 上開發，Linux 和 macOS 版本的測試較少。如果遇到問題，歡迎提交 issue。",
    issue: "提交 issue",
  },
  notFound: {
    title: "找不到頁面",
    body: "這個網址沒有頁面。",
    home: "返回首頁",
  },
};
