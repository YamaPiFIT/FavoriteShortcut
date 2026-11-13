## ダウンロード

`FavoriteShortcut.exe` … これ1つで動きます（.NET 不要）
`FavoriteShortcut-portable.zip` … 上記 + README / LICENSE
`FavoriteShortcutRs.exe` … **Rust 版**（今回追加）。これ1つで動きます
`FavoriteShortcutRs-portable.zip` … 上記 + Rust 版の説明 / LICENSE / ライセンス表記

以前のバージョンから更新する場合は **EXE を上書きするだけ**です。登録したデータはそのまま残ります。

## v1.4.0 の変更点

- **Rust 版（`FavoriteShortcutRs.exe`）を追加しました** … これまでの版（C# 版）と同じ機能・同じ画面構成を、Rust で作り直したものです
  - 登録内容・アイコン・バックアップ・設定は、C# 版と**同じものをそのまま使います**（移行は不要です）。`FavoriteShortcut.exe` と同じフォルダに置けば、同じデータが表示されます
  - C# 版と同時には起動しません。切り替えるときは、タスクトレイのメニューの「終了」で片方を終了してから、もう片方を起動してください
  - Windows へのサインイン時の自動起動は、設定画面で最後にオンにした方が起動します
  - 画面の描画に GPU（DirectX 12 / Vulkan / OpenGL）を使います
  - 文字の描画やスクロールの感触など、見た目の細部は C# 版と少し異なります
  - 初めて使う前に、念のため C# 版でエクスポートかバックアップ（「⋯」→「今すぐバックアップを作成」）をしておくと安心です
- C# 版（`FavoriteShortcut.exe`）の動作に変更はありません（バージョン番号のみ）

Changes: added a Rust version of the app (`FavoriteShortcutRs.exe`). It has the same features and screens as the existing C# version and uses the same data (shortcuts, icons, backups and settings) as is, so no migration is needed: just place it next to `FavoriteShortcut.exe`. The two versions do not run at the same time; quit one from the tray menu before starting the other. The C# version is unchanged except for its version number.

## 動作環境

Windows 10 (1809 以降) / Windows 11 の 64bit

初回起動時の「WindowsによってPCが保護されました」は、未署名のアプリのため出るものです。
**詳細情報** → **実行** で起動できます。

<!-- SHA256 -->

MIT License / [第三者ライセンス表記](https://github.com/YamaPiFIT/FavoriteShortcut/blob/main/THIRD-PARTY-NOTICES.md) / [Rust 版のライセンス表記](https://github.com/YamaPiFIT/FavoriteShortcut/blob/main/rust/THIRD-PARTY-LICENSES.md)
