## ダウンロード

`FavoriteShortcut.exe` … これ1つで動きます（.NET 不要）
`FavoriteShortcut-portable.zip` … 上記 + README / LICENSE

以前のバージョンから更新する場合は **EXE を上書きするだけ**です。登録したデータはそのまま残ります。

## v1.3.1 の変更点

会社の PC などで、起動や画面の表示に時間がかかることがある問題への対策です。

- **アプリからサイトへの通信をやめました** … Web サイトのアイコンは、ブラウザが保存しているものだけを使います。会社のネットワークで起きやすい「通信を始める前の待ち時間（プロキシの自動検出など）」で、起動や画面の表示が遅くなることがなくなります
  - まだブラウザで開いたことのないサイトは、一度開くまで既定のアイコンです。このアプリから開けば、少し待って自動でアイコンが付きます
  - すでに付いているアイコンはそのまま残ります
  - 登録画面のボタンは「ブラウザから取得」に変わりました
- **共有フォルダを待たないようにしました** … 共有フォルダ（`\\サーバー\...` やネットワークドライブ）上のフォルダ・ファイルは、いったん標準のアイコンで表示し、本来のアイコンは裏で取得して差し替えます。登録画面でのパスの確認も裏で行うので、サーバーの応答が遅くても画面が止まりません

Changes: the app no longer connects to websites. Site icons come only from your browser's saved icons, so startup no longer waits on the network (such as proxy auto-detection on corporate networks). Shortcuts on shared folders and network drives no longer freeze the screen: their icons and path checks now load in the background.

## v1.3.0 の新機能（v1.2.1 以前から更新する方へ）

- **毎日の自動バックアップ** … 登録内容が変わった日だけ、1 日 1 回自動でバックアップします（最新 14 回分を保持）。設定 → データ でオン / オフできます
- **トレイに最近使った項目** … トレイアイコンの右クリックメニューに、最近使った 10 件が並びます
- **クリップボードから登録** … URL やパスをコピーして、管理画面で `Ctrl + V`、ランチャーで `Ctrl + N`、またはトレイのメニューから登録できます。どのアプリからでも使えるキーも、設定 → ランチャー で割り当てられます
- **テーマの「自動」** … Windows のライト / ダーク設定に合わせて切り替わります。設定 → 表示 → テーマ で選べます
- **ドラッグ中の強調表示** … ショートカットをフォルダへドラッグするとき、移動先のフォルダが強調されます

New in v1.3.0: daily automatic backups, recent items in the tray menu, add from clipboard (Ctrl+V / Ctrl+N), automatic light/dark theme, and highlighted drop targets.

## 動作環境

Windows 10 (1809 以降) / Windows 11 の 64bit

初回起動時の「WindowsによってPCが保護されました」は、未署名のアプリのため出るものです。
**詳細情報** → **実行** で起動できます。

<!-- SHA256 -->

MIT License / [第三者ライセンス表記](https://github.com/YamaPiFIT/FavoriteShortcut/blob/main/THIRD-PARTY-NOTICES.md)
