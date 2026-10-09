## ダウンロード

`FavoriteShortcut.exe` … これ1つで動きます（.NET 不要）
`FavoriteShortcut-portable.zip` … 上記 + README / LICENSE

以前のバージョンから更新する場合は **EXE を上書きするだけ**です。登録したデータはそのまま残ります。

## v1.3.2 の変更点

- **ショートカットを開くときに、アプリが止まらないようにしました** … ネットワークの状態が悪いとき、ショートカットを開いた後にランチャー（`Ctrl + Space`）や画面の反応が遅くなることがありました
  - 原因: Windows は、サイトやフォルダを開く処理の中でセキュリティの確認（サイトのゾーン判定、署名の失効確認、共有フォルダへの問い合わせなど）を行います。ネットワークの状態が悪いとこの確認が応答を待ち続け、その間アプリの画面の処理も一緒に止まっていました
  - 対策: 開く処理を、画面とは別に行うようにしました。Windows が確認している間も、ランチャーや画面は普段どおり動きます
  - Windows の確認そのものは省いていません（安全性は変わりません）

Changes: opening a shortcut no longer freezes the app. When the network is slow, Windows' own security checks while opening sites and folders (zone checks, certificate revocation checks, file server queries) could keep the app from responding, including the Ctrl+Space launcher. Shortcuts now open on a separate thread, and Windows' checks still run as before.

## 動作環境

Windows 10 (1809 以降) / Windows 11 の 64bit

初回起動時の「WindowsによってPCが保護されました」は、未署名のアプリのため出るものです。
**詳細情報** → **実行** で起動できます。

<!-- SHA256 -->

MIT License / [第三者ライセンス表記](https://github.com/YamaPiFIT/FavoriteShortcut/blob/main/THIRD-PARTY-NOTICES.md)
