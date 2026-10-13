import type { Messages } from "./en";

export const ja: Messages = {
  meta: {
    description:
      "AI アシスタントを搭載し、自分の Cloudflare アカウントを通じてエンドツーエンド暗号化で同期できる、オープンソースのデスクトップ SSH クライアント",
    tagline: "AI アシスタント搭載のオープンソース SSH クライアント",
    downloadTitle: "Hatoba をダウンロード",
    downloadLead: (version) => `Windows、Linux、macOS 向けの Hatoba ${version} をダウンロードできます。`,
  },
  nav: {
    home: "Hatoba ホーム",
    features: "機能",
    download: "ダウンロード",
    docs: "ドキュメント",
    github: "GitHub",
    language: "言語",
    skip: "本文へスキップ",
  },
  platforms: { windows: "Windows", linux: "Linux", macos: "macOS" },
  hero: {
    downloadFor: (platform) => `${platform} 版をダウンロード`,
    allDownloads: "すべてのダウンロード",
    github: "GitHub で見る",
    available: (version) => `バージョン ${version}（Windows、Linux、macOS）`,
    alt: "Hatoba の AI アシスタントがターミナルの横で nginx の 502 エラーの原因を見つけて修正したところ",
  },
  ai: {
    title: "すべてのターミナルの横に AI アシスタント",
    body: "ターミナルの横のパネルで問題を伝えると、AI アシスタントが画面を読み、コマンドを実行し、ホスト上のファイルを読んで編集して、原因を突き止めて修正します。",
    points: [
      "コマンドはどれもあなたの承認を待ち、実行前に編集することもできます。自動承認では、あなたが選んだモデルが安全なものを承認します。",
      "変更したファイルにはすべて差分が表示され、元に戻せます。",
      "/ でコマンドやスキルを呼び出し、@ でホスト上のファイルを添付できます。独自のスキルや MCP サーバーも追加できます。",
      "OpenAI 互換または Anthropic の API に対応したモデルなら、ローカルモデルも含めて使えます。",
    ],
    alt: "アシスタントが実行しようとしているコマンドの承認カード",
  },
  sync: {
    title: "自分の Cloudflare アカウントで同期",
    body: "Hatoba が Worker と D1 データベースをあなたの Cloudflare アカウントにデプロイし、ホスト、鍵、AI との会話をデバイス間で同期します。個人での利用なら、たいてい無料プランの範囲に収まります。",
    points: [
      "エンドツーエンド暗号化：データはデバイスを離れる前に暗号化されるため、Worker とデータベースにあるのは暗号文だけです。",
      "Hatoba のサーバーはありません：同期はすべてあなたのアカウントの中で動きます。",
      "同期は任意：使わなくても、すべての機能がオフラインで動きます。",
    ],
    alt: "接続方法、デバイス一覧、暗号化の説明が表示されたクラウド同期ページ",
  },
  essentials: {
    title: "SSH クライアントに必要なものはすべて",
    items: [
      {
        term: "ホスト",
        text: "グループ、タグ、お気に入り、あいまい検索。~/.ssh/config を読み込めるほか、ProxyJump や SOCKS5、HTTP プロキシ経由で接続でき、ssh user@host と入力すればホストを保存せずに接続できます。",
      },
      {
        term: "ダッシュボード",
        text: "保存したすべてのホストの状態、CPU、メモリ、ディスク、ネットワークを 1 ページで確認できます。SSH 経由で読み取るので、サーバーには何もインストールしません。",
      },
      {
        term: "ターミナル",
        text: "タブ、トゥルーカラー、日本語などの入力メソッド、検索、クリックできるリンクに対応。複数行を貼り付ける前には確認します。",
      },
      { term: "SFTP", text: "ターミナルの横のファイルパネル。ドラッグ＆ドロップでアップロードでき、ダウンロードはいつでもキャンセルできます。" },
      { term: "ポートフォワーディング", text: "ローカルフォワーディングを接続と同時に開始できます。" },
      {
        term: "鍵",
        text: "パスワード、Ed25519・ECDSA・RSA 鍵、ssh-agent、キーボードインタラクティブ認証（2 要素認証）に対応。OpenSSH、PEM、PuTTY の鍵を読み込むことも、新しく生成することもできます。",
      },
    ],
    alt: "各ホストの CPU、メモリ、ディスクの使用状況を表示したダッシュボード",
  },
  security: {
    title: "データはデバイス上で暗号化",
    body: "保管庫はマスターパスワードで開きます。鍵はマスターパスワードから Argon2id で導出され、すべての項目は保存や同期の前に AES-256-GCM で暗号化されます。マスターパスワードを忘れ、リカバリーコードもなくした場合、データは復元できません。",
    openSource: "Hatoba は MIT ライセンスのオープンソースなので、データの扱い方を自分で確かめられます。",
    link: "セキュリティモデルを読む（英語）",
  },
  cta: {
    title: "Hatoba を使ってみる",
    body: "無料のオープンソース。Windows 10・11、x86_64 の Linux、Apple シリコン搭載の Mac（試験的）に対応しています。",
  },
  footer: {
    releases: "リリース",
    report: "問題を報告",
    security: "セキュリティポリシー",
    contributing: "コントリビュート",
    license: "Hatoba は MIT ライセンスで公開しています。",
  },
  download: {
    released: (version, date) => `バージョン ${version}（${date} リリース）`,
    notes: "リリースノート",
    preview: (version) => `バージョン ${version} のプレビュー版も公開しています。`,
    previewLink: "プレビュー版を見る",
    thisDevice: "このデバイス向け",
    requirements: {
      windows: "Windows 10 または 11（x64）。",
      linux: "x86_64。同期は認証情報を Secret Service に保存するため、GNOME Keyring、KWallet などの Secret Service プロバイダーが必要です。",
      macos: "Apple シリコン、macOS 13 以降。このビルドは試験的なものです。",
    },
    files: {
      nsis: "インストーラー（.exe）",
      deb: "Debian・Ubuntu（.deb）",
      appimage: "その他のディストリビューション（AppImage）",
      dmg: "ディスクイメージ（.dmg）",
    },
    size: (megabytes) => `${megabytes} MB`,
    install: "インストール方法",
    sha256: "SHA-256",
    copy: "コピー",
    copied: "コピーしました",
    checksums: "に、このリリースのすべてのファイルのチェックサムがあります。",
    allReleases: "すべてのリリース",
    source: "ソースからビルド",
    testing: "Hatoba は主に Windows で開発しているため、Linux 版と macOS 版はテストが十分ではありません。うまく動かない場合は Issue でお知らせください。",
    issue: "Issue を作成",
  },
  notFound: {
    title: "ページが見つかりません",
    body: "このアドレスにはページがありません。",
    home: "ホームに戻る",
  },
};
